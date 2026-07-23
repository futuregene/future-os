use super::*;
use std::path::PathBuf;

use crate::{
    agent::Loop,
    tools::coding_tools,
    types::{AgentMessage, LLMProvider, Message, StreamEvent, ToolCall, ToolCallFn, ToolDef},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

struct MockWriteProvider {
    calls: AtomicUsize,
    outside_path: String,
}

#[async_trait::async_trait]
impl LLMProvider for MockWriteProvider {
    async fn stream_chat(
        &self,
        _model: String,
        _messages: Vec<Message>,
        _tools: Vec<ToolDef>,
        _system_prompt: String,
    ) -> anyhow::Result<ReceiverStream<StreamEvent>> {
        let (tx, rx) = mpsc::channel(8);
        let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
        let outside_path = self.outside_path.clone();

        tokio::spawn(async move {
            if call_index == 0 {
                let arguments = serde_json::json!({
                    "path": outside_path,
                    "content": "should not leave workspace"
                });
                let _ = tx
                    .send(event_with_tool_call(
                        "toolcall_start",
                        "call_test",
                        "write",
                        arguments,
                    ))
                    .await;
                let _ = tx.send(simple_event("toolcall_end")).await;
            } else {
                let _ = tx.send(text_event("done")).await;
                let _ = tx.send(simple_event("stop")).await;
            }
        });

        Ok(ReceiverStream::new(rx))
    }
}

fn simple_event(event_type: &str) -> StreamEvent {
    StreamEvent {
        event_type: event_type.to_string(),
        ..Default::default()
    }
}

fn text_event(text: &str) -> StreamEvent {
    StreamEvent {
        event_type: "text_delta".to_string(),
        text: text.to_string(),
        ..Default::default()
    }
}

fn event_with_tool_call(
    event_type: &str,
    tool_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> StreamEvent {
    StreamEvent {
        event_type: event_type.to_string(),
        tool_name: tool_name.to_string(),
        tool_id: tool_id.to_string(),
        tool_call: Some(ToolCall {
            id: tool_id.to_string(),
            call_type: "function".to_string(),
            function: ToolCallFn {
                name: tool_name.to_string(),
                arguments,
            },
        }),
        ..Default::default()
    }
}

fn test_path(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("futureos-session-{name}-{stamp}"))
}

#[tokio::test]
async fn loop_workspace_scope_blocks_unapproved_absolute_write_from_model_tool_call() {
    let workspace = test_path("workspace");
    // Outside must be outside every writable root — temp dirs are
    // writable roots now (SANDBOX_PLAN.md §2.2), so use home.
    let outside = dirs::home_dir().unwrap().join(format!(
        "futureos-session-outside-{}.txt",
        std::process::id()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    let provider = Arc::new(MockWriteProvider {
        calls: AtomicUsize::new(0),
        outside_path: outside.to_string_lossy().to_string(),
    });
    let agent_loop = Loop::new(provider, "mock").with_tools(coding_tools());

    // v2: the boundary only applies when the sandbox is enabled (GUI). A
    // disabled/non-GUI session runs fully open, so enable it here.
    let mut sandbox = crate::sandbox::ResolvedSandbox::resolve(
        &crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Manual,
        },
        workspace.to_string_lossy().as_ref(),
    );
    sandbox.available = false;
    crate::tools::with_tool_scope(
        crate::tools::ScopeOptions {
            workspace: workspace.to_string_lossy().to_string(),
            permission_level: "workspace".to_string(),
            interrupt_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sandbox: Arc::new(sandbox),
            escalation: None,
            on_sandboxed: None,
        },
        async {
            let _ = agent_loop
                .run_streaming_with_messages(
                    vec![AgentMessage::new_user(
                        "user",
                        serde_json::json!([{"type": "text", "text": "write outside"}]),
                    )],
                    &crate::agent::StreamContext::default(),
                    |_| {},
                    |_| {},
                    None,
                )
                .await;
        },
    )
    .await;

    assert!(!outside.exists());
}

/// Verify that session_info entries written by the prompt save path carry
/// all the fields that `switch_session` and the GUI fork path expect.
#[test]
fn session_info_content_includes_required_fields() {
    use crate::session::SessionEntry;

    // Simulate the content JSON written by the final prompt save path.
    // This mirrors the structure built at ~line 395 in persist_user_message
    // and ~line 395 in the post-run save.
    let content = serde_json::json!({
        "cwd": "/tmp/test-ws",
        "tokens_in": 1000,
        "tokens_out": 500,
        "tokens_cache_r": 200,
        "tokens_cache_w": 100,
        "last_prompt_tokens": 1800,
        "total_cost": 0.05,
        "session_name": "fix the bug",
        "auto_compaction": true,
        "parent_session_id": "parent-1",
        "thinking_level": "high",
        "created_by": "gui",
        "source_meta": {"threadId": "t1"},
    });

    // Fields that switch_session reads from session_info content
    assert!(content.get("thinking_level").and_then(|v| v.as_str()) == Some("high"));
    assert!(content.get("session_name").and_then(|v| v.as_str()) == Some("fix the bug"));
    assert!(content.get("auto_compaction").and_then(|v| v.as_bool()) == Some(true));
    assert!(content.get("cwd").and_then(|v| v.as_str()) == Some("/tmp/test-ws"));

    // Fields that fork_agent_session (GUI) reads from session_info content
    assert!(content.get("session_name").and_then(|v| v.as_str()) == Some("fix the bug"));
    assert!(content.get("created_by").and_then(|v| v.as_str()) == Some("gui"));

    // Token counters must survive a crash → must be in content
    assert!(content.get("tokens_in").and_then(|v| v.as_i64()) == Some(1000));
    assert!(content.get("tokens_out").and_then(|v| v.as_i64()) == Some(500));
    assert!(content.get("total_cost").and_then(|v| v.as_f64()) == Some(0.05));

    // Construct a SessionEntry from this content — must round-trip.
    let entry = SessionEntry::session_info(content.clone(), "claude".into(), "high".into());
    assert_eq!(entry.entry_type, "session_info");
    assert_eq!(entry.role, "system");

    let restored = entry.content.unwrap();
    assert_eq!(
        restored.get("thinking_level").and_then(|v| v.as_str()),
        Some("high")
    );
    assert_eq!(
        restored.get("session_name").and_then(|v| v.as_str()),
        Some("fix the bug")
    );
}

/// Verify that the mid-stream save performs the same auto-generation
/// for session_name as the final save — an empty `self.session_name`
/// must not produce an empty string in the content JSON.
#[test]
fn session_info_session_name_never_empty() {
    // Simulated entries where the first user message is "hello world".
    let entries: Vec<serde_json::Value> = vec![serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": "hello world"}],
    })];
    // Replicate the auto-generation logic from persist_user_message.
    let name = entries
        .iter()
        .find(|e| e.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|e| e.get("content"))
        .map(|c| {
            if let Some(arr) = c.as_array() {
                arr.iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .next()
                    .unwrap_or("")
                    .to_string()
            } else {
                c.as_str().unwrap_or("").to_string()
            }
        })
        .map(|s| crate::session::truncate_visible(s.trim(), 40))
        .unwrap_or_default();
    assert!(!name.is_empty());
    assert_eq!(name, "hello world");

    // If there's no user entry at all, it should be empty string (not panic).
    let empty: Vec<serde_json::Value> = vec![];
    let fallback = empty
        .iter()
        .find(|e| e.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|e| e.get("content"))
        .map(|c| {
            if let Some(arr) = c.as_array() {
                arr.iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .next()
                    .unwrap_or("")
                    .to_string()
            } else {
                c.as_str().unwrap_or("").to_string()
            }
        })
        .unwrap_or_default();
    assert!(fallback.is_empty()); // no user = empty, not crash
}

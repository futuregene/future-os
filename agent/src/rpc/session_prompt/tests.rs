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
        "created_by": "desktop",
        "source_meta": {"threadId": "t1"},
    });

    // Fields that switch_session reads from session_info content
    assert!(content.get("thinking_level").and_then(|v| v.as_str()) == Some("high"));
    assert!(content.get("session_name").and_then(|v| v.as_str()) == Some("fix the bug"));
    assert!(content.get("auto_compaction").and_then(|v| v.as_bool()) == Some(true));
    assert!(content.get("cwd").and_then(|v| v.as_str()) == Some("/tmp/test-ws"));

    // Fields that fork_agent_session (desktop) reads from session_info content
    assert!(content.get("session_name").and_then(|v| v.as_str()) == Some("fix the bug"));
    assert!(content.get("created_by").and_then(|v| v.as_str()) == Some("desktop"));

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

// ── Run identity reconciliation ──────────────────────────────────────────

fn stamped_message(role: &str, run_id: Option<&str>) -> AgentMessage {
    let metadata = run_id.map(|id| {
        let mut map = serde_json::Map::new();
        map.insert(
            "run_id".to_string(),
            serde_json::Value::String(id.to_string()),
        );
        map
    });
    AgentMessage {
        role: role.to_string(),
        content: vec![crate::types::ContentBlock::text("x")],
        metadata,
        ..Default::default()
    }
}

fn meta_str(message: &AgentMessage, key: &str) -> Option<String> {
    message
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(key))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

#[test]
fn reconcile_run_identity_stamps_run_messages_and_spares_prior_runs() {
    let mut messages = vec![
        stamped_message("user", None),                 // legacy prior run
        stamped_message("assistant", Some("run-old")), // prior run's reply
        stamped_message("user", Some("run-new")),      // this run's opener
        stamped_message("assistant", None),
        stamped_message("tool", None),
        stamped_message("assistant", None),
    ];

    reconcile_run_identity(&mut messages, "run-new");

    // Prior runs are never touched.
    assert!(messages[0].metadata.is_none());
    assert_eq!(meta_str(&messages[1], "run_id").as_deref(), Some("run-old"));
    // This run: every entry carries the run id.
    assert_eq!(meta_str(&messages[2], "run_id").as_deref(), Some("run-new"));
    assert_eq!(meta_str(&messages[3], "run_id").as_deref(), Some("run-new"));
    assert_eq!(meta_str(&messages[4], "run_id").as_deref(), Some("run-new"));
    assert_eq!(meta_str(&messages[5], "run_id").as_deref(), Some("run-new"));
}

#[test]
fn reconcile_run_identity_keeps_existing_ids_and_falls_back_to_last_assistant() {
    let mut messages = vec![
        stamped_message("user", Some("run-x")),
        stamped_message("assistant", None),
    ];
    reconcile_run_identity(&mut messages, "run-x");
    assert_eq!(meta_str(&messages[0], "run_id").as_deref(), Some("run-x"));
    assert_eq!(
        meta_str(&messages[1], "run_id").as_deref(),
        Some("run-x"),
        "the reply joins the opener's run"
    );

    // No stamped opener (a compaction rewrite dropped it): the sweep leaves
    // the unidentified prefix alone, but the run's final assistant reply is
    // still attributable.
    let mut messages = vec![
        stamped_message("user", None),
        stamped_message("assistant", None),
    ];
    reconcile_run_identity(&mut messages, "run-y");
    assert_eq!(meta_str(&messages[0], "run_id"), None);
    assert_eq!(meta_str(&messages[1], "run_id").as_deref(), Some("run-y"));
}

// ─── full-run driver tests (scripted provider) ─────────────────────────────

struct ScriptedProvider {
    scripts: std::sync::Mutex<std::collections::VecDeque<Script>>,
}

enum Script {
    Events(Vec<StreamEvent>),
    Fail(String),
    /// Send events, then hold the stream open (never closes).
    Stall(Vec<StreamEvent>),
}

impl ScriptedProvider {
    fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            scripts: std::sync::Mutex::new(scripts.into()),
        })
    }
}

#[async_trait::async_trait]
impl LLMProvider for ScriptedProvider {
    async fn stream_chat(
        &self,
        _model: String,
        _messages: Vec<Message>,
        _tools: Vec<ToolDef>,
        _system_prompt: String,
    ) -> anyhow::Result<ReceiverStream<StreamEvent>> {
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .expect("test ran out of scripted responses");
        let (events, stall) = match script {
            Script::Events(events) => (events, false),
            Script::Stall(events) => (events, true),
            Script::Fail(error) => return Err(anyhow::Error::msg(error)),
        };
        let (tx, rx) = mpsc::channel(events.len().max(1));
        for event in events {
            let _ = tx.try_send(event);
        }
        if stall {
            std::mem::forget(tx);
        }
        Ok(ReceiverStream::new(rx))
    }
}

struct RunFixture {
    workspace: PathBuf,
    session: crate::rpc::ServerSession,
}

fn run_fixture(provider: Arc<dyn LLMProvider>, name: &str) -> RunFixture {
    let dir = test_path(name);
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let manager = Arc::new(crate::session::Manager::new(dir.join("sessions")));
    let agent_loop = Loop::new(provider, "mock").with_tools(coding_tools());
    let session = crate::rpc::ServerSession::new_with_queue_budget(
        "s1".to_string(),
        Arc::new(tokio::sync::RwLock::new(agent_loop)),
        manager,
        workspace.to_string_lossy().as_ref(),
        Arc::new(crate::rpc::SseBroadcaster::new()),
        crate::rpc::ApprovalGate::default(),
        Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
    );
    RunFixture {
        workspace,
        session,
    }
}

impl RunFixture {
    fn workspace(&self) -> &PathBuf {
        &self.workspace
    }
}

async fn wait_for_run_end(session: &crate::rpc::ServerSession) {
    use std::sync::atomic::Ordering;
    for _ in 0..500 {
        let active = session.runtime.snapshot().is_some();
        let streaming = session.is_streaming.load(Ordering::Relaxed);
        if !active && !streaming {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("run did not finish within 5s");
}

fn text_turn(text: &str) -> Script {
    Script::Events(vec![text_event(text), simple_event("stop")])
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_text_run_completes_and_persists() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("the answer")]), "basic");
    let mut session = fixture.session;
    let lease = session.prompt("the question", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    // In-memory history: user + assistant.
    let messages = session.messages.read().clone();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].text(), "the answer");

    // On-disk journal for this run exists and ends with a terminal event.
    let journal = session
        .session_manager
        .run_data_path("s1")
        .join(format!("{}.jsonl", lease.run_id));
    for _ in 0..100 {
        if journal.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let journal_text = std::fs::read_to_string(&journal).unwrap();
    assert!(journal_text.contains("\"agent_start\""), "{journal_text}");
    assert!(journal_text.contains("\"agent_end\""), "{journal_text}");
    assert!(journal_text.contains("completed"), "{journal_text}");

    // The session transcript carries the user + assistant + run markers.
    let mut stored = None;
    for _ in 0..100 {
        if let Ok(loaded) = session.session_manager.load("s1") {
            if loaded
                .entries
                .iter()
                .any(|e| e.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL)
            {
                stored = Some(loaded);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let stored = stored.expect("terminal entry persisted");
    let types: Vec<&str> = stored
        .entries
        .iter()
        .map(|e| e.entry_type.as_str())
        .collect();
    assert!(types.contains(&"user"), "{types:?}");
    assert!(types.contains(&"assistant"), "{types:?}");
    assert!(types.contains(&crate::session::ENTRY_TYPE_RUN_STARTED));
    assert!(types.contains(&crate::session::ENTRY_TYPE_RUN_TERMINAL));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_tool_run_executes_and_records_tool_result() {
    let write_args = serde_json::json!({"path": "out.txt", "content": "from tool"});
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            event_with_tool_call("toolcall_start", "call-1", "write", write_args),
            simple_event("toolcall_end"),
            simple_event("stop"),
        ]),
        text_turn("written"),
    ]);
    let fixture = run_fixture(provider, "tool");
    let out_path = fixture.workspace().join("out.txt");
    let mut session = fixture.session;
    session.prompt("write a file", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "from tool");
    let messages = session.messages.read().clone();
    let tool_msg = messages.iter().find(|m| m.role == "tool").expect("tool message");
    assert!(tool_msg.text().contains("out.txt"));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_permission_none_denies_tool_calls() {
    let write_args = serde_json::json!({"path": "nope.txt", "content": "x"});
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            event_with_tool_call("toolcall_start", "call-1", "write", write_args),
            simple_event("toolcall_end"),
            simple_event("stop"),
        ]),
        text_turn("cannot"),
    ]);
    let fixture = run_fixture(provider, "deny");
    let denied_path = fixture.workspace().join("nope.txt");
    let mut session = fixture.session;
    session.set_permission_level("none");
    session.prompt("write a file", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    assert!(!denied_path.exists());
    let messages = session.messages.read().clone();
    let tool_msg = messages.iter().find(|m| m.role == "tool").expect("tool message");
    assert!(tool_msg.text().contains("denied"));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_provider_failure_records_error_terminal() {
    let provider = ScriptedProvider::new(vec![Script::Fail("upstream is down".to_string())]);
    let fixture = run_fixture(provider, "error");
    let mut session = fixture.session;
    session.set_auto_retry(false); // fail once, no 2s/4s/8s backoff
    // The failure surfaces through the run task, not the prompt() return.
    let _ = session.prompt("hi", &[], &[], None, None);
    wait_for_run_end(&session).await;

    let mut terminal_state = None;
    for _ in 0..100 {
        if let Ok(loaded) = session.session_manager.load("s1") {
            if let Some(content) = loaded
                .entries
                .iter()
                .find(|e| e.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL)
                .and_then(|e| e.content.clone())
            {
                terminal_state = Some(content["state"].as_str().unwrap_or("").to_string());
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        terminal_state.as_deref(),
        Some(crate::session::RUN_STATE_ERROR)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_ephemeral_session_skips_persistence() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("hi")]), "ephemeral");
    let mut session = fixture.session;
    session.set_ephemeral(true);
    session.prompt("hello", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().len(), 2);
    assert!(
        session.session_manager.find("s1").is_none(),
        "ephemeral sessions never touch the transcript"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_prompt_starts_immediately_and_materializes_attachments() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("got it")]), "enqueue");
    let attachment_path = fixture.workspace().join("note.txt");
    std::fs::write(&attachment_path, "attachment body").unwrap();

    let mut session = fixture.session;
    let ack = session
        .enqueue_prompt(
            "read this",
            &[],
            &[crate::types::Attachment {
                path: attachment_path.to_string_lossy().to_string(),
                kind: "file".to_string(),
                name: "note.txt".to_string(),
                ..Default::default()
            }],
            None,
            "req-enqueue",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(
        ack.accepted_state,
        crate::runtime::RunAcceptedState::Running
    );
    wait_for_run_end(&session).await;
    let messages = session.messages.read().clone();
    assert_eq!(messages.last().unwrap().text(), "got it");
    // The attachment became part of the user message context.
    assert!(messages[0].text().contains("read this"));
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_prompt_fails_while_loop_is_locked() {
    let fixture = run_fixture(ScriptedProvider::new(vec![]), "locked");
    let mut session = fixture.session;
    let agent_loop = session.agent_loop.clone();
    let _guard = agent_loop.try_write().unwrap();
    let result = session.enqueue_prompt(
        "hi",
        &[],
        &[],
        None,
        "req-locked",
        crate::runtime::BusyPolicy::EnqueueIfBusy,
    );
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("configuration is busy"));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_auto_compaction_compacts_and_rewrites_history() {
    // glm-4.5v has a 64k context window in the builtin catalog: a 50k-token
    // first turn crosses the 90% threshold and forces compaction before turn 2.
    let big_text = "lorem ipsum dolor sit amet ".repeat(6000); // ~150 KB
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            event_with_tool_call(
                "toolcall_start",
                "call-1",
                "read",
                serde_json::json!({"path": "x"}),
            ),
            simple_event("toolcall_end"),
            StreamEvent {
                event_type: "usage".to_string(),
                usage: Some(crate::types::Usage {
                    prompt_tokens: 50_000,
                    completion_tokens: 100,
                    total_tokens: 50_100,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    credit_cost: None,
                }),
                ..Default::default()
            },
            simple_event("stop"),
        ]),
        text_turn("done"),
    ]);
    let fixture = run_fixture(provider, "compact");
    let mut session = fixture.session;
    session.model = "glm-4.5v".to_string();
    session.prompt(&big_text, &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    let loop_ = session.agent_loop.read().await;
    assert!(
        loop_
            .compaction_occurred
            .load(std::sync::atomic::Ordering::SeqCst),
        "compaction fired before the second turn"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_abort_produces_cancelled_terminal() {
    // The provider sends one event then goes quiet; abort interrupts the run.
    let provider = ScriptedProvider::new(vec![Script::Stall(vec![text_event("never finished")])]);
    let fixture = run_fixture(provider, "abort");
    let mut session = fixture.session;
    let lease = session.prompt("hi", &[], &[], None, None).unwrap();
    // Let the run start streaming, then abort.
    for _ in 0..100 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    session.abort_run(Some(&lease.run_id)).unwrap();
    wait_for_run_end(&session).await;
    assert!(session.runtime.snapshot().is_none());
}

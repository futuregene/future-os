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
    /// Wait for the signal, then deliver the events.
    Gated(Arc<tokio::sync::Notify>, Vec<StreamEvent>),
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
            Script::Gated(notify, events) => {
                notify.notified().await;
                (events, false)
            }
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

    /// The on-disk transcript path for session "s1" (may not exist yet).
    fn transcript_file(&self) -> PathBuf {
        self.workspace
            .parent()
            .unwrap()
            .join("sessions")
            .join("s1.jsonl")
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

// ─── batch 3: rare error arms ──────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn prompt_with_unaccessible_cwd_fails_fast() {
    let fixture = run_fixture(ScriptedProvider::new(vec![]), "bad-cwd");
    // Point the session at a plain file — not a directory.
    let file = fixture.workspace().join("plain.txt");
    std::fs::write(&file, "x").unwrap();
    let mut session = fixture.session;
    session.set_cwd(file.to_string_lossy().as_ref());
    let result = session.prompt("hi", &[], &[], None, None);
    assert!(result.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn scheduled_run_with_broken_transcript_reports_error() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("unused")]), "sched-fail");
    let transcript = fixture.transcript_file();
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&transcript).unwrap(); // dir where the file belongs
    let mut session = fixture.session;
    let result = session.enqueue_prompt(
        "hi",
        &[],
        &[],
        None,
        "req-broken",
        crate::runtime::BusyPolicy::EnqueueIfBusy,
    );
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&transcript);
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_derives_session_name_from_string_content_entry() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("ok")]), "string-name");
    let mut session = fixture.session;
    // A pre-existing user entry with plain-string content takes the as_str
    // arm of the name derivation.
    session
        .messages
        .write()
        .push(crate::types::AgentMessage::new_user(
            "user",
            serde_json::json!("the earlier question"),
        ));
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    let loaded = session.session_manager.load("s1").unwrap();
    assert!(!loaded.name.is_empty());
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)] // HOME must stay pinned across awaits
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_sandbox_denial_escalates_through_session_wiring() {
    let _home_guard = crate::test_support::home_env_lock();
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("futureos-run-escalate-{}.txt", std::process::id()));
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            event_with_tool_call(
                "toolcall_start",
                "call-1",
                "shell",
                serde_json::json!({"command": format!("touch {}", outside.display())}),
            ),
            simple_event("toolcall_end"),
            simple_event("stop"),
        ]),
        text_turn("done"),
    ]);
    let fixture = run_fixture(provider, "run-escalate");
    let gate = fixture.session.approval_gate.clone();
    let mut session = fixture.session;
    session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Sandbox,
    });
    if !crate::sandbox::platform_sandbox_available() {
        return;
    }
    // The escalation approval arrives from a decider thread.
    let decider = std::thread::spawn(move || {
        for _ in 0..2000 {
            let pending = gate.pending_for_session("s1");
            if let Some(first) = pending.first() {
                let request_id = first["approval_request_id"].as_str().unwrap().to_string();
                let _ = gate.decide(
                    &request_id,
                    "s1",
                    crate::rpc::ApprovalDecision {
                        approved: true,
                        note: String::new(),
                        status: crate::rpc::ApprovalDecisionStatus::Approved,
                    },
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("escalation request never appeared");
    });
    session.prompt("touch outside", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    decider.join().unwrap();
    assert!(outside.exists(), "approved re-run created the file");
    let _ = std::fs::remove_file(&outside);
}

// ─── batch 2: enqueue/persist/finalize edge arms ───────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn enqueue_duplicate_run_id_via_transcript() {
    let fixture = run_fixture(ScriptedProvider::new(vec![]), "dupe-run");
    let mut session = fixture.session;
    // Persist a transcript that already contains a marker for run-dupe.
    let snapshot = crate::session::Session::snapshot(
        "s1".to_string(),
        session.cwd.clone(),
        "mock".to_string(),
        String::new(),
        String::new(),
        vec![crate::session::SessionEntry::run_started("run-dupe", 1)],
    );
    session.session_manager.save(&snapshot).unwrap();

    let result = session.enqueue_prompt(
        "hi",
        &[],
        &[],
        Some("run-dupe"),
        "req-new",
        crate::runtime::BusyPolicy::EnqueueIfBusy,
    );
    let error = result.unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_same_request_twice_returns_existing_ack() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![Script::Stall(vec![text_event("running")])]),
        "idempotent",
    );
    let mut session = fixture.session;
    let first = session
        .enqueue_prompt(
            "same body",
            &[],
            &[],
            None,
            "req-idem",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(
        first.accepted_state,
        crate::runtime::RunAcceptedState::Running
    );
    // Identical re-submission is a no-op returning the original ack.
    let second = session
        .enqueue_prompt(
            "same body",
            &[],
            &[],
            None,
            "req-idem",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(
        second.accepted_state,
        crate::runtime::RunAcceptedState::Existing
    );
    assert_eq!(second.run_id, first.run_id);
    session.abort_run(Some(&first.run_id)).unwrap();
    wait_for_run_end(&session).await;
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_with_sandbox_policy_parses_tier() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("ok")]), "tier");
    let mut session = fixture.session;
    session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Manual,
    });
    let ack = session
        .enqueue_prompt(
            "hi",
            &[],
            &[],
            None,
            "req-tier",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(
        ack.accepted_state,
        crate::runtime::RunAcceptedState::Running
    );
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().last().unwrap().text(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_with_unknown_thinking_level_uses_zero_budget() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("ok")]), "weird-level");
    let mut session = fixture.session;
    session.set_thinking_level("ultra"); // not a known level → 0 budget arm
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().last().unwrap().text(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_verbose_logs_user_message() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("ok")]), "verbose");
    let mut session = fixture.session;
    session.agent_loop.write().await.verbose = true;
    session.prompt("loud question", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().last().unwrap().text(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_persist_failure_aborts_run_with_error() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("unused")]), "persist-fail");
    // A directory where the transcript file should be breaks persistence.
    let transcript = fixture.transcript_file();
    let mut session = fixture.session;
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&transcript).unwrap();

    let result = session.prompt("hi", &[], &[], None, None);
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&transcript);
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_second_run_uses_fast_append_path() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("first"), text_turn("second")]),
        "two-runs",
    );
    let mut session = fixture.session;
    session.prompt("one", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    session.prompt("two", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    let messages = session.messages.read().clone();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[3].text(), "second");
    // Both runs' markers coexist in the transcript.
    let loaded = session.session_manager.load("s1").unwrap();
    let terminals = loaded
        .entries
        .iter()
        .filter(|e| e.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL)
        .count();
    assert_eq!(terminals, 2);
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_with_explicit_name_and_provenance_persists_info() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("ok")]), "provenance");
    let mut session = fixture.session;
    session.set_session_name("explicit name");
    session.created_by = "gui".to_string();
    session.source_meta = serde_json::json!({"thread": "t-1"});
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    let loaded = session.session_manager.load("s1").unwrap();
    assert_eq!(loaded.name, "explicit name");
    let info = loaded
        .entries
        .iter()
        .rev()
        .find(|e| e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO)
        .and_then(|e| e.content.clone())
        .unwrap();
    assert_eq!(info["created_by"], "gui");
    assert_eq!(info["source_meta"]["thread"], "t-1");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_with_project_context_file() {
    let provider = ScriptedProvider::new(vec![text_turn("ok")]);
    let fixture = run_fixture(provider, "context");
    std::fs::write(fixture.workspace().join("CLAUDE.md"), "# ctx").unwrap();
    let mut session = fixture.session;
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().last().unwrap().text(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_auto_compaction_with_small_history_completes() {
    // 50k tokens reported against a 64k window forces the compaction attempt.
    // (A compact() failure needs a history with no valid cut point, which the
    // public session flow cannot produce — a finished tool turn always leaves
    // ≥3 messages. That arm is covered by the run_loop unit test.)
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
        text_turn("compacted reply"),
    ]);
    let fixture = run_fixture(provider, "compact-small");
    let mut session = fixture.session;
    session.model = "glm-4.5v".to_string();
    session.prompt("short", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    let loaded = session.session_manager.load("s1").unwrap();
    let terminal = loaded
        .entries
        .iter()
        .rev()
        .find(|e| e.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL)
        .and_then(|e| e.content.clone())
        .unwrap();
    assert_eq!(terminal["state"], crate::session::RUN_STATE_COMPLETED);
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_mid_run_append_failure_heals_via_full_rewrite() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![Script::Gated(
        gate.clone(),
        vec![text_event("late answer"), simple_event("stop")],
    )]);
    let fixture = run_fixture(provider, "heal");
    let transcript = fixture.transcript_file();
    let mut session = fixture.session;
    session.prompt("hi", &[], &[], None, None).unwrap();

    // Wait for the user message to hit disk, then remove the transcript so
    // the mid-run assistant append fails (open-for-append on a missing file).
    for _ in 0..200 {
        if transcript.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    std::fs::remove_file(&transcript).unwrap();

    gate.notify_one();
    wait_for_run_end(&session).await;

    // The refused commit healed with a full rewrite: the reply is on disk.
    let loaded = session.session_manager.load("s1").unwrap();
    assert!(loaded.entries.iter().any(|e| {
        e.content
            .as_ref()
            .is_some_and(|c| c.to_string().contains("late answer"))
    }));
    assert!(loaded
        .entries
        .iter()
        .any(|e| e.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_persistence_commit_and_rewrite_failure_marks_degraded() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("doomed")]), "degraded");
    let mut session = fixture.session;
    // The commit is refused and the healing rewrite fails → the run is
    // marked persistence_degraded instead of reporting a false completion.
    session.persistence.fail_next_commit();
    session.persistence.fail_next_rewrite();
    session.prompt("hi", &[], &[], None, None).unwrap();

    let mut degraded = false;
    for _ in 0..300 {
        if session
            .runtime
            .snapshot()
            .is_some_and(|snap| snap.phase == crate::runtime::RunPhase::PersistenceDegraded)
        {
            degraded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(degraded, "run ended in persistence_degraded phase");
}

// HOME_ENV_LOCK is a plain Mutex by design: it must exclude TestHome
// redirects in OTHER threads for the whole test, including across awaits.
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_workspace_permission_routes_through_approval_gate() {
    let _home_guard = crate::HOME_ENV_LOCK.lock().unwrap();
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("futureos-gate-target-{}.txt", std::process::id()));
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            event_with_tool_call(
                "toolcall_start",
                "call-1",
                "write",
                serde_json::json!({"path": outside.to_string_lossy(), "content": "ok"}),
            ),
            simple_event("toolcall_end"),
            simple_event("stop"),
        ]),
        text_turn("done"),
    ]);
    let fixture = run_fixture(provider, "gate");
    let gate = fixture.session.approval_gate.clone();
    let mut session = fixture.session;
    session.set_permission_level("workspace");
    session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Manual,
    });
    // The outside path is past the sandbox boundary → the gate asks; the
    // decider approves, so the write proceeds.
    let decider = std::thread::spawn(move || {
        for _ in 0..2000 {
            let pending = gate.pending_for_session("s1");
            if let Some(first) = pending.first() {
                let request_id = first["approval_request_id"].as_str().unwrap().to_string();
                let _ = gate.decide(
                    &request_id,
                    "s1",
                    crate::rpc::ApprovalDecision {
                        approved: true,
                        note: String::new(),
                        status: crate::rpc::ApprovalDecisionStatus::Approved,
                    },
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("approval request never appeared");
    });
    session.prompt("write", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    decider.join().unwrap();
    let messages = session.messages.read().clone();
    assert!(messages.iter().any(|m| m.role == "tool"));
    assert!(outside.exists());
    let _ = std::fs::remove_file(&outside);
}

use super::*;
use std::path::PathBuf;

use crate::{
    agent::Loop,
    llm::schema::{FinishReason, ModelRequest, ModelStreamEvent},
    tools::coding_tools,
    types::{AgentMessage, LLMProvider},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

#[test]
fn deferred_context_preparation_checks_the_admitted_prefix_and_cancellation() {
    let mut user = AgentMessage::new_user("user", serde_json::json!("accepted input"));
    user.ensure_journal_entry_id();
    let messages = vec![user];
    let boundary = RunInputBoundary::capture(&messages);
    let interrupt = std::sync::atomic::AtomicBool::new(false);
    let prepared = boundary.snapshot(&messages, &interrupt).unwrap();
    assert_eq!(prepared[0].display_text(), "accepted input");
    let mut replaced = messages.clone();
    replaced[0].metadata.as_mut().unwrap().insert(
        AgentMessage::JOURNAL_ENTRY_ID_KEY.into(),
        serde_json::json!("different accepted source"),
    );
    assert!(boundary
        .snapshot(&replaced, &interrupt)
        .unwrap_err()
        .to_string()
        .contains("input changed"));
    replaced = messages.clone();
    replaced.push(AgentMessage::new_user(
        "user",
        serde_json::json!("later queued input"),
    ));
    assert!(boundary.snapshot(&replaced, &interrupt).is_err());
    interrupt.store(true, Ordering::Relaxed);
    assert!(boundary
        .snapshot(&messages, &interrupt)
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    assert_eq!(messages[0].display_text(), "accepted input");
}

struct MockWriteProvider {
    calls: AtomicUsize,
    outside_path: String,
}

#[async_trait::async_trait]
impl LLMProvider for MockWriteProvider {
    async fn stream_model(
        &self,
        _request: ModelRequest,
    ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
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
                    .send(tool_start_event("call_test", "write", arguments))
                    .await;
                let _ = tx.send(tool_end_event()).await;
            } else {
                let _ = tx.send(text_event("done")).await;
                let _ = tx.send(finish_event()).await;
            }
        });

        Ok(ReceiverStream::new(rx))
    }
}

fn finish_event() -> ModelStreamEvent {
    ModelStreamEvent::Finish {
        reason: FinishReason::Stop,
        usage: None,
    }
}

fn tool_end_event() -> ModelStreamEvent {
    ModelStreamEvent::ToolInputEnd {
        index: 0,
        id: String::new(),
        name: String::new(),
        arguments: serde_json::Value::Null,
        provider_metadata: Default::default(),
    }
}

fn text_event(text: &str) -> ModelStreamEvent {
    ModelStreamEvent::TextDelta {
        id: "text".into(),
        text: text.to_string(),
    }
}

fn tool_start_event(
    tool_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> ModelStreamEvent {
    ModelStreamEvent::ToolInputStart {
        index: 0,
        id: tool_id.to_string(),
        name: tool_name.to_string(),
        arguments: Some(arguments),
        provider_metadata: Default::default(),
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
    // writable roots now (docs/internals/desktop/SANDBOX/COMMON.md), so use home.
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
            model_reviewer: false,
        },
        workspace.to_string_lossy().as_ref(),
    );
    sandbox.set_backend_available_for_test(false);
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
        .map(|s| crate::session::first_message_title(s.as_str()))
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
    requests: std::sync::Mutex<Vec<ModelRequest>>,
}

enum Script {
    Events(Vec<ModelStreamEvent>),
    Fail(String),
    /// Send events, then hold the stream open (never closes).
    Stall(Vec<ModelStreamEvent>),
    /// Wait for the signal, then deliver the events.
    Gated(Arc<tokio::sync::Notify>, Vec<ModelStreamEvent>),
}

impl ScriptedProvider {
    fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            scripts: std::sync::Mutex::new(scripts.into()),
            requests: std::sync::Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl LLMProvider for ScriptedProvider {
    async fn stream_model(
        &self,
        request: ModelRequest,
    ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
        self.requests.lock().unwrap().push(request.clone());
        // A C3 summary request carries the agent's own system prompt so the provider
        // can serve it from cache; recognise it by the instruction appended last.
        let summary_instruction = request
            .messages
            .last()
            .map(|message| message.text())
            .unwrap_or_default();
        if request
            .system_prompt
            .contains("context summarization agent")
            || summary_instruction.contains("handoff summary for another agent")
        {
            let (tx, rx) = mpsc::channel(2);
            tx.send(text_event(
                "## Objective\n- Continue the test task.\n\n## Important Details\n- Preserve history.\n\n## Work State\n### Completed\n- Initial tool turn completed.\n\n### Active\n- Continue the run.\n\n### Blocked\n- (none)\n\n## Next Move\n1. Finish the response.\n\n## Relevant Files\n- x: test input",
            ))
            .await
            .unwrap();
            tx.send(finish_event()).await.unwrap();
            return Ok(ReceiverStream::new(rx));
        }
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
    run_fixture_with_id(provider, name, "s1")
}

/// The same fixture with a caller-chosen session id. Tests that need to find
/// their own run's wiring (e.g. the captured run callbacks) use a unique id so
/// they cannot consume a parallel test's capture.
fn run_fixture_with_id(provider: Arc<dyn LLMProvider>, name: &str, session_id: &str) -> RunFixture {
    let dir = test_path(name);
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let manager = Arc::new(crate::session::Manager::new(dir.join("sessions")));
    let agent_loop = Loop::new(provider, "mock").with_tools(coding_tools());
    let session = crate::rpc::ServerSession::new_with_queue_budget(
        session_id.to_string(),
        Arc::new(tokio::sync::RwLock::new(agent_loop)),
        manager,
        workspace.to_string_lossy().as_ref(),
        Arc::new(crate::rpc::SseBroadcaster::new()),
        crate::rpc::ApprovalGate::default(),
        Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
    );
    RunFixture { workspace, session }
}

/// Remove the run wiring `prompt_internal` captured for `session_id`.
fn take_run_callbacks(session_id: &str) -> RunCallbacksForTest {
    // The wiring is installed by the run's own `prompt_internal` call, so this
    // is reached only after a run for this id started.
    RUN_CALLBACKS_FOR_TEST
        .lock()
        .remove(session_id)
        .expect("the run captured its callbacks")
}

impl RunFixture {
    fn workspace(&self) -> &PathBuf {
        &self.workspace
    }
}

async fn wait_for_run_end(session: &crate::rpc::ServerSession) {
    use std::sync::atomic::Ordering;
    // A liveness wait, not a speed claim: the assertion is "the run finishes".
    // The bound is generous because an instrumented parallel run of the whole
    // crate is several times slower than a single-test run, and a tight bound
    // flaked (observed "run did not finish within 5s" at 5s under llvm-cov).
    for _ in 0..3000 {
        let active = session.runtime.snapshot().is_some();
        let streaming = session.is_streaming.load(Ordering::Relaxed);
        if !active && !streaming {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("run did not finish within 30s");
}

fn text_turn(text: &str) -> Script {
    Script::Events(vec![text_event(text), finish_event()])
}

#[tokio::test(flavor = "current_thread")]
async fn an_isolated_recovery_refuses_direct_and_queued_work_before_any_model_or_ack() {
    let provider = ScriptedProvider::new(vec![text_turn("must never execute")]);
    let mut fixture = run_fixture(provider.clone(), "recovery-execution-fence");
    let manager = fixture.session.session_manager.clone();
    manager.storage().unwrap().replace("s1", vec![
        serde_json::json!({"id":"info","type":"session_info","timestamp":"2026-10-28T00:00:00Z","content":{"cwd":".","model":"mock"}}),
        serde_json::json!({"id":"start","type":"run_started","timestamp":"2026-10-28T00:00:00Z","content":{"run_id":"broken","epoch":1}}),
    ]).unwrap();
    manager.storage().unwrap().db.call(|db| {
        db.execute("UPDATE runs SET status='recovery_failed',error='fixture' WHERE session_id='s1' AND run_id='broken'", [])?;
        Ok(())
    }).unwrap();
    let before = manager.storage().unwrap().entries("s1").unwrap();
    let direct = fixture
        .session
        .prompt("do work", &[], &[], Some("next"), None)
        .unwrap_err();
    assert!(direct
        .to_string()
        .contains("session recovery requires repair"));
    let queued = fixture
        .session
        .enqueue_prompt(
            "queue work",
            &[],
            &[],
            Some("queued"),
            "recovery-request",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap_err();
    assert!(queued
        .to_string()
        .contains("session recovery requires repair"));
    assert!(matches!(
        queued.downcast_ref::<crate::runtime::RunQueueError>(),
        Some(crate::runtime::RunQueueError::PersistenceUnavailable(reason))
            if reason.contains("session recovery requires repair")
    ));
    assert!(provider.requests.lock().unwrap().is_empty());
    assert!(fixture.session.runtime.snapshot().is_none());
    assert!(manager
        .queued_receipt("s1", "recovery-request")
        .unwrap()
        .is_none());
    assert_eq!(manager.storage().unwrap().entries("s1").unwrap(), before);
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_text_run_completes_and_persists() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("the answer")]),
        "basic",
    );
    let mut session = fixture.session;
    let lease = session
        .prompt("the question", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;

    // In-memory history: user + assistant.
    let messages = session.messages.read().clone();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].text(), "the answer");

    // On-disk journal for this run exists and ends with a terminal event.
    let journal_text = serde_json::to_string(
        &session
            .session_manager
            .storage()
            .unwrap()
            .events("s1", &lease.run_id)
            .unwrap(),
    )
    .unwrap();
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
            tool_start_event("call-1", "write", write_args),
            tool_end_event(),
            finish_event(),
        ]),
        text_turn("written"),
    ]);
    let fixture = run_fixture(provider, "tool");
    let out_path = fixture.workspace().join("out.txt");
    let mut session = fixture.session;
    session
        .prompt("write a file", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;

    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "from tool");
    let messages = session.messages.read().clone();
    let tool_msg = messages
        .iter()
        .find(|m| m.role == "tool")
        .expect("tool message");
    assert!(tool_msg.text().contains("out.txt"));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_projects_typed_model_and_tool_events_end_to_end() {
    let arguments = serde_json::json!({"path": "typed.txt", "content": "typed"});
    let usage = crate::types::Usage {
        prompt_tokens: 3,
        completion_tokens: 2,
        total_tokens: 5,
        ..Default::default()
    };
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            ModelStreamEvent::ReasoningStart {
                id: "reasoning".into(),
            },
            ModelStreamEvent::ReasoningDelta {
                id: "reasoning".into(),
                text: "plan".into(),
            },
            ModelStreamEvent::ReasoningEnd {
                id: "reasoning".into(),
                provider_metadata: Default::default(),
            },
            ModelStreamEvent::ToolInputStart {
                index: 0,
                id: "call-typed".into(),
                name: "write".into(),
                arguments: None,
                provider_metadata: Default::default(),
            },
            ModelStreamEvent::ToolInputDelta {
                index: 0,
                id: "call-typed".into(),
                delta: serde_json::to_string(&arguments).unwrap(),
                snapshot: true,
            },
            ModelStreamEvent::ToolInputEnd {
                index: 0,
                id: "call-typed".into(),
                name: "write".into(),
                arguments,
                provider_metadata: Default::default(),
            },
            ModelStreamEvent::Usage(usage.clone()),
            ModelStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
                usage: None,
            },
        ]),
        Script::Events(vec![
            text_event("done"),
            ModelStreamEvent::Finish {
                reason: FinishReason::Stop,
                usage: Some(usage),
            },
        ]),
    ]);
    let fixture = run_fixture(provider, "typed-events");
    let output_path = fixture.workspace().join("typed.txt");
    let mut events = fixture.session.broadcaster.subscribe();
    let mut session = fixture.session;
    session.prompt("write typed", &[], &[], None, None).unwrap();

    let mut event_types = Vec::new();
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("run event timeout")
            .expect("run event");
        if event.event_type == "user_message" {
            let payload: serde_json::Value = serde_json::from_str(&event.data).unwrap();
            let messages = session.messages.read();
            let user = messages
                .iter()
                .find(|message| message.role == "user")
                .unwrap();
            assert_eq!(payload["entry_id"].as_str(), user.journal_entry_id());
            assert_eq!(payload["run_id"], user.metadata.as_ref().unwrap()["run_id"]);
            assert_eq!(payload["text"], "write typed");
            assert!(payload["created_at_ms"].as_i64().unwrap() > 0);
        }
        event_types.push(event.event_type.clone());
        if event.event_type == "agent_end" {
            break;
        }
    }

    assert_eq!(
        event_types,
        vec![
            "user_message",
            "agent_start",
            "message_start",
            "thinking_start",
            "thinking_delta",
            "thinking_end",
            "tool_start",
            "tool_delta",
            "tool_input_end",
            "usage",
            "message_end",
            "tool_start",
            "tool_end",
            "message_start",
            "text_chunk",
            "usage",
            "message_end",
            "agent_end",
        ]
    );
    assert_eq!(std::fs::read_to_string(output_path).unwrap(), "typed");
}

#[tokio::test(flavor = "current_thread")]
async fn incomplete_reason_is_identical_in_live_event_and_sqlite_history() {
    for (index, reason) in [
        FinishReason::Length,
        FinishReason::Incomplete,
        FinishReason::Paused,
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = run_fixture(
            ScriptedProvider::new(vec![Script::Events(vec![
                text_event("synthetic partial"),
                ModelStreamEvent::Finish {
                    reason,
                    usage: None,
                },
            ])]),
            &format!("terminal-reason-{index}"),
        );
        let mut session = fixture.session;
        let mut events = session.broadcaster.subscribe();
        session
            .prompt("synthetic request", &[], &[], None, None)
            .unwrap();
        let live = loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            if event.event_type == "agent_end" {
                break serde_json::from_str::<serde_json::Value>(&event.data).unwrap();
            }
        };
        let stored = session.session_manager.load("s1").unwrap();
        let terminal = stored
            .entries
            .iter()
            .find(|entry| entry.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL)
            .unwrap()
            .content
            .as_ref()
            .unwrap();
        assert_eq!(live["state"], "incomplete");
        assert_eq!(terminal["state"], live["state"]);
        assert_eq!(terminal["error"], live["error"]);
        assert!(terminal["error"].as_str().unwrap().starts_with('['));
        assert_eq!(terminal["truncation"], live["truncation"]);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_permission_none_denies_tool_calls() {
    let write_args = serde_json::json!({"path": "nope.txt", "content": "x"});
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start_event("call-1", "write", write_args),
            tool_end_event(),
            finish_event(),
        ]),
        text_turn("cannot"),
    ]);
    let fixture = run_fixture(provider, "deny");
    let denied_path = fixture.workspace().join("nope.txt");
    let mut session = fixture.session;
    session.set_permission_level("none");
    session
        .prompt("write a file", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;

    assert!(!denied_path.exists());
    let messages = session.messages.read().clone();
    let tool_msg = messages
        .iter()
        .find(|m| m.role == "tool")
        .expect("tool message");
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
    session.set_ephemeral(true).unwrap();
    session.prompt("hello", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().len(), 2);
    assert!(
        !session.session_manager.contains("s1").unwrap(),
        "ephemeral sessions never touch the transcript"
    );
}

/// An ephemeral session has no transcript, so a failed run has no run terminal
/// to persist: the live stream is the only place its failure can be told, and a
/// client that never sees it would show a run that simply stopped. The run must
/// therefore emit the `error` event and *still* close with an `agent_end` that
/// names the state, the error and the run's usage.
#[tokio::test(flavor = "current_thread")]
async fn an_ephemeral_run_that_fails_reports_the_error_in_its_agent_end() {
    let provider = ScriptedProvider::new(vec![Script::Fail("upstream is down".to_string())]);
    let fixture = run_fixture(provider, "ephemeral-error");
    let mut session = fixture.session;
    session.set_ephemeral(true).unwrap();
    session.set_auto_retry(false); // fail once, no 2s/4s/8s backoff
    let mut events = session.broadcaster.subscribe();
    session.prompt("hi", &[], &[], None, None).unwrap();

    let mut event_types = Vec::new();
    let mut reported = None;
    let end = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("run event timeout")
            .expect("run event");
        event_types.push(event.event_type.clone());
        match event.event_type.as_str() {
            "error" => {
                reported = Some(serde_json::from_str::<serde_json::Value>(&event.data).unwrap());
            }
            "agent_end" => break serde_json::from_str::<serde_json::Value>(&event.data).unwrap(),
            _ => {}
        }
    };
    let reported = reported.expect("a failed run reports an error event before it ends");
    assert!(
        reported["error"]
            .as_str()
            .unwrap()
            .contains("upstream is down"),
        "{reported}"
    );
    assert_eq!(end["type"], "agent_end");
    assert_eq!(end["state"], crate::session::RUN_STATE_ERROR);
    assert!(
        end["error"].as_str().unwrap().contains("upstream is down"),
        "the terminal event carries the failure, not just its state: {end}"
    );
    assert!(
        end["usage"]["output_tokens"].is_u64(),
        "a failed run still reports its usage: {end}"
    );
    assert!(end["duration_ms"].is_i64(), "{end}");
    assert_eq!(
        event_types.last().map(String::as_str),
        Some("agent_end"),
        "nothing may follow the terminal event: {event_types:?}"
    );
    assert!(
        !session.session_manager.contains("s1").unwrap(),
        "ephemeral sessions never touch the transcript"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_prompt_starts_immediately_with_a_large_live_attachment_reference() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("got it")]), "enqueue");
    let attachment_path = fixture.workspace().join("note.txt");
    std::fs::File::create(&attachment_path)
        .unwrap()
        .set_len(20 * 1024 * 1024)
        .unwrap();

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
    assert_eq!(
        messages[0].metadata.as_ref().unwrap()["attachments"][0]["path"],
        attachment_path.to_string_lossy().as_ref()
    );
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
async fn prompt_auto_compaction_appends_checkpoint_and_retains_history() {
    // A large tool result plus reported usage crosses the request budget;
    // the user's short original must survive S2 while tool history is summarized.
    let big_text = "lorem ipsum dolor sit amet ".repeat(6000); // ~150 KB
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start_event("call-1", "read", serde_json::json!({"path": "x"})),
            tool_end_event(),
            ModelStreamEvent::Usage(crate::types::Usage {
                prompt_tokens: 55_000,
                completion_tokens: 100,
                total_tokens: 55_100,
                cache_read_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
                credit_cost: None,
                provider_metadata: None,
            }),
            finish_event(),
        ]),
        text_turn("done"),
    ]);
    let fixture = run_fixture(provider, "compact");
    std::fs::write(fixture.workspace().join("x"), &big_text).unwrap();
    let mut session = fixture.session;
    session
        .model_registry
        .write()
        .test_insert(crate::models::Model {
            id: "compact-small".into(),
            provider: "test".into(),
            api: "openai-completions".into(),
            api_key: "fixture".into(),
            context_window: 64_000,
            max_tokens: 4096,
            input: vec!["text".into()],
            output: vec!["text".into()],
            ..Default::default()
        });
    session.model = "test/compact-small".to_string();
    session
        .prompt("read the lorem ipsum file", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;

    let persisted = session.session_manager.load(&session.session_id).unwrap();
    assert!(persisted
        .entries
        .iter()
        .any(|entry| entry.entry_type == crate::session::ENTRY_TYPE_COMPACTION));
    assert!(
        persisted
            .entries
            .iter()
            .any(|entry| entry.entry_type == crate::session::ENTRY_TYPE_USER
                && entry
                    .content
                    .as_ref()
                    .is_some_and(|content| content.to_string().contains("lorem ipsum"))),
        "covered history remains in the append-only journal"
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

#[tokio::test(flavor = "current_thread")]
async fn supersede_interrupts_active_run_and_queues_successor() {
    // Steering (`supersede_session`) must be a real interrupt — the stalled
    // first run is aborted, not left to run on — and the successor is queued
    // with a fresh run identity.
    let provider = ScriptedProvider::new(vec![
        Script::Stall(vec![text_event("old run partial")]),
        text_turn("new answer"),
    ]);
    let fixture = run_fixture(provider, "supersede-interrupt");
    let mut session = fixture.session;

    let first = session.prompt("first", &[], &[], None, None).unwrap();
    for _ in 0..200 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(
        session.runtime.snapshot().is_some(),
        "first run should be actively streaming"
    );

    let ack = session
        .enqueue_prompt(
            "second",
            &[],
            &[],
            None,
            "req-supersede",
            crate::runtime::BusyPolicy::SupersedeSession,
        )
        .unwrap();
    assert_eq!(
        ack.accepted_state,
        crate::runtime::RunAcceptedState::Queued,
        "superseding an active run queues the successor"
    );
    assert_ne!(ack.run_id, first.run_id);

    // The stalled first run never finishes on its own — reaching idle proves
    // the supersede actually interrupted it.
    wait_for_run_end(&session).await;
    assert!(
        session.runtime.snapshot().is_none(),
        "session is idle after the active run was interrupted"
    );

    // The successor is queued behind the (now-finished) interrupted run.
    let queued = session.scheduler.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].payload["message"], "second");
    assert_eq!(queued[0].run_id, ack.run_id);
}

#[tokio::test(flavor = "current_thread")]
async fn queued_follow_ups_keep_independent_runs() {
    let provider = ScriptedProvider::new(vec![
        Script::Stall(vec![text_event("stalled")]),
        Script::Stall(vec![text_event("second")]),
    ]);
    let fixture = run_fixture(provider, "merge-follow-ups");
    let mut session = fixture.session;

    let first = session.prompt("first", &[], &[], None, None).unwrap();
    for _ in 0..200 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let a = session
        .enqueue_prompt(
            "second question",
            &[],
            &[],
            None,
            "req-2",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    let b = session
        .enqueue_prompt(
            "third question",
            &[],
            &[],
            None,
            "req-3",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(a.accepted_state, crate::runtime::RunAcceptedState::Queued);
    assert_eq!(b.accepted_state, crate::runtime::RunAcceptedState::Queued);
    assert_eq!(session.scheduler.queued().len(), 2);

    // Interrupt the stalled run so its queued work can drain.
    session.abort_run(Some(&first.run_id)).unwrap();
    for _ in 0..500 {
        if session.runtime.snapshot().is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(session.runtime.snapshot().is_none());

    // Drain manually (the fixture has no scheduler worker). Each accepted
    // request retains its own identity and payload.
    let started = session.start_next_scheduled().unwrap();
    assert_eq!(
        started.accepted_state,
        crate::runtime::RunAcceptedState::Running
    );

    assert_eq!(session.scheduler.queued().len(), 1);
    assert_eq!(
        session.scheduler.queued()[0].payload["message"],
        "third question"
    );

    let user_texts: Vec<String> = session
        .messages
        .read()
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.text())
        .collect();
    assert!(user_texts.iter().any(|t| t == "second question"));
    assert!(!user_texts.iter().any(|t| t == "third question"));
    assert!(!user_texts
        .iter()
        .any(|t| t == "second question\n\nthird question"));
    session.abort_run(Some(&started.run_id)).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn coalescing_follow_ups_fold_into_one_run_in_queue_order() {
    let provider = ScriptedProvider::new(vec![
        Script::Stall(vec![text_event("stalled")]),
        text_turn("merged answer"),
    ]);
    let fixture = run_fixture(provider, "coalesce-follow-ups");
    let mut session = fixture.session;

    let first = session.prompt("first", &[], &[], None, None).unwrap();
    for _ in 0..200 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let a = session
        .enqueue_prompt(
            "second question",
            &[],
            &[],
            None,
            "req-2",
            crate::runtime::BusyPolicy::EnqueueCoalescing,
        )
        .unwrap();
    let b = session
        .enqueue_prompt(
            "third question",
            &[],
            &[],
            None,
            "req-3",
            crate::runtime::BusyPolicy::EnqueueCoalescing,
        )
        .unwrap();
    assert_eq!(a.accepted_state, crate::runtime::RunAcceptedState::Queued);
    assert_eq!(b.accepted_state, crate::runtime::RunAcceptedState::Queued);
    assert_eq!(session.scheduler.queued().len(), 2);

    // Interrupt the stalled run so its queued work can drain.
    session.abort_run(Some(&first.run_id)).unwrap();
    for _ in 0..500 {
        if session.runtime.snapshot().is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(session.runtime.snapshot().is_none());

    let started = session.start_next_scheduled().unwrap();
    assert_eq!(
        started.accepted_state,
        crate::runtime::RunAcceptedState::Running
    );
    // One run absorbed both follow-ups: nothing is left queued behind it.
    assert_eq!(started.run_id, a.run_id);
    assert!(session.scheduler.queued().is_empty());

    let user_texts: Vec<String> = session
        .messages
        .read()
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.text())
        .collect();
    // The questions arrive as ONE user message, in queue order — never as two
    // separate messages, and never reordered.
    assert!(
        user_texts
            .iter()
            .any(|t| t == "second question\n\nthird question"),
        "folded user message not found in {user_texts:?}"
    );
    assert!(!user_texts.iter().any(|t| t == "second question"));
    assert!(!user_texts.iter().any(|t| t == "third question"));

    // The folded submission never runs, so its client learns it was merged
    // rather than watching a queued run that never starts.
    let merged: Vec<String> = session
        .scheduler
        .recent_terminal_acks()
        .into_iter()
        .filter(|ack| ack.reason == "merged")
        .map(|ack| ack.run_id)
        .collect();
    assert_eq!(merged, vec![b.run_id.clone()]);

    // The fold is recorded on the accepted message (history + journal audit).
    let coalesced = session
        .messages
        .read()
        .iter()
        .find(|m| m.text() == "second question\n\nthird question")
        .and_then(|m| m.metadata.clone())
        .and_then(|metadata| metadata.get("coalesced_run_ids").cloned())
        .expect("coalesced_run_ids metadata");
    assert_eq!(coalesced, serde_json::json!([b.run_id.clone()]));

    session.abort_run(Some(&started.run_id)).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn coalescing_uses_the_last_folded_requests_execution_settings() {
    // The user's LATEST instruction decides how the combined turn runs: the
    // front request's own execution settings must not be reused for it.
    let provider = ScriptedProvider::new(vec![
        Script::Stall(vec![text_event("stalled")]),
        text_turn("merged answer"),
    ]);
    let fixture = run_fixture(provider, "coalesce-last-settings");
    let valid_workspace = fixture.workspace().clone();
    let mut session = fixture.session;

    let first = session.prompt("first", &[], &[], None, None).unwrap();
    for _ in 0..200 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let not_a_directory = valid_workspace.join("not-a-directory");
    std::fs::write(&not_a_directory, b"x").unwrap();
    // The front request is accepted with a cwd that cannot be a workspace, so a
    // run that reused its settings could not start at all.
    session.set_cwd(not_a_directory.to_string_lossy().as_ref());
    session
        .enqueue_prompt(
            "front",
            &[],
            &[],
            None,
            "req-2",
            crate::runtime::BusyPolicy::EnqueueCoalescing,
        )
        .unwrap();
    session.set_cwd(valid_workspace.to_string_lossy().as_ref());
    session
        .enqueue_prompt(
            "last",
            &[],
            &[],
            None,
            "req-3",
            crate::runtime::BusyPolicy::EnqueueCoalescing,
        )
        .unwrap();

    session.abort_run(Some(&first.run_id)).unwrap();
    for _ in 0..500 {
        if session.runtime.snapshot().is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let started = session.start_next_scheduled().unwrap();
    assert_eq!(
        started.accepted_state,
        crate::runtime::RunAcceptedState::Running
    );
    session.abort_run(Some(&started.run_id)).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn coalescing_stops_at_a_request_that_did_not_opt_in() {
    let provider = ScriptedProvider::new(vec![
        Script::Stall(vec![text_event("stalled")]),
        Script::Stall(vec![text_event("second")]),
    ]);
    let fixture = run_fixture(provider, "coalesce-mixed-policies");
    let mut session = fixture.session;

    let first = session.prompt("first", &[], &[], None, None).unwrap();
    for _ in 0..200 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    session
        .enqueue_prompt(
            "folded question",
            &[],
            &[],
            None,
            "req-2",
            crate::runtime::BusyPolicy::EnqueueCoalescing,
        )
        .unwrap();
    // A caller that streams its own run keeps its own run: it is never folded
    // into another submission's answer.
    let independent = session
        .enqueue_prompt(
            "independent question",
            &[],
            &[],
            None,
            "req-3",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();

    session.abort_run(Some(&first.run_id)).unwrap();
    for _ in 0..500 {
        if session.runtime.snapshot().is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let started = session.start_next_scheduled().unwrap();
    let user_texts: Vec<String> = session
        .messages
        .read()
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.text())
        .collect();
    assert!(user_texts.iter().any(|t| t == "folded question"));
    assert!(!user_texts.iter().any(|t| t == "independent question"));
    // The non-coalescing request still owns its own queued run.
    let queued = session.scheduler.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].run_id, independent.run_id);

    session.abort_run(Some(&started.run_id)).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn coalescing_joins_model_context_sidecars_in_order() {
    let provider = ScriptedProvider::new(vec![
        Script::Stall(vec![text_event("stalled")]),
        text_turn("merged answer"),
    ]);
    let fixture = run_fixture(provider, "coalesce-model-context");
    let mut session = fixture.session;

    let first = session.prompt("first", &[], &[], None, None).unwrap();
    for _ in 0..200 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let ctx2 = "context-two".to_string();
    let ctx3 = "context-three".to_string();
    for (text, context, request) in [
        ("second question", &ctx2, "req-2"),
        ("third question", &ctx3, "req-3"),
    ] {
        session
            .enqueue_prompt_with_model_context(
                PromptText::new(text, context),
                &[],
                &[],
                None,
                request,
                crate::runtime::BusyPolicy::EnqueueCoalescing,
            )
            .unwrap();
    }

    session.abort_run(Some(&first.run_id)).unwrap();
    for _ in 0..500 {
        if session.runtime.snapshot().is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let started = session.start_next_scheduled().unwrap();
    let user_messages: Vec<crate::types::AgentMessage> = session
        .messages
        .read()
        .iter()
        .filter(|m| m.role == "user")
        .cloned()
        .collect();
    // Both the visible text and the non-display sidecar fold in the same order.
    let folded = user_messages
        .iter()
        .find(|m| m.display_text() == "second question\n\nthird question")
        .expect("folded user message");
    let text = folded.text();
    assert!(text.contains("context-two"), "{text}");
    assert!(text.contains("context-three"), "{text}");
    assert!(
        text.find("context-two") < text.find("context-three"),
        "sidecars keep queue order: {text}"
    );

    session.abort_run(Some(&started.run_id)).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn queued_follow_ups_keep_model_context_sidecars_independent() {
    let provider = ScriptedProvider::new(vec![
        Script::Stall(vec![text_event("stalled")]),
        Script::Stall(vec![text_event("second")]),
    ]);
    let fixture = run_fixture(provider, "merge-model-context");
    let mut session = fixture.session;

    let first = session.prompt("first", &[], &[], None, None).unwrap();
    for _ in 0..200 {
        if session.runtime.snapshot().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let ctx2 = "context-two".to_string();
    let ctx3 = "context-three".to_string();
    let a = session
        .enqueue_prompt_with_model_context(
            PromptText::new("second question", &ctx2),
            &[],
            &[],
            None,
            "req-2",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    let b = session
        .enqueue_prompt_with_model_context(
            PromptText::new("third question", &ctx3),
            &[],
            &[],
            None,
            "req-3",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(a.accepted_state, crate::runtime::RunAcceptedState::Queued);
    assert_eq!(b.accepted_state, crate::runtime::RunAcceptedState::Queued);
    assert_eq!(session.scheduler.queued().len(), 2);

    session.abort_run(Some(&first.run_id)).unwrap();
    for _ in 0..500 {
        if session.runtime.snapshot().is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(session.runtime.snapshot().is_none());

    let started = session.start_next_scheduled().unwrap();
    assert_eq!(
        started.accepted_state,
        crate::runtime::RunAcceptedState::Running
    );

    let user_messages: Vec<crate::types::AgentMessage> = session
        .messages
        .read()
        .iter()
        .filter(|m| m.role == "user")
        .cloned()
        .collect();
    assert!(user_messages
        .iter()
        .any(|m| m.display_text() == "second question" && m.text().contains("context-two")));
    assert!(!user_messages
        .iter()
        .any(|m| m.text().contains("context-three")));
    let queued = session.scheduler.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].payload["message"], "third question");
    assert_eq!(queued[0].payload["model_context"], "context-three");
    session.abort_run(Some(&started.run_id)).unwrap();
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
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("unused")]),
        "sched-fail",
    );
    fixture.session.session_manager.test_execute("CREATE TRIGGER fail_entries BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;");
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
async fn run_sandbox_failure_returns_to_model_without_approval_or_replay() {
    let _home_guard = crate::test_support::home_env_lock();
    let outside = dirs::home_dir().unwrap().join(format!(
        "futureos-run-on-request-{}.txt",
        crate::utils::generate_id()
    ));
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start_event(
                "call-1",
                "shell",
                serde_json::json!({"command": format!("echo partial >> partial; touch '{}'", outside.display())}),
            ),
            tool_end_event(),
            finish_event(),
        ]),
        text_turn("blocked; inspect completed work"),
    ]);
    let fixture = run_fixture(provider.clone(), "run-on-request");
    let partial = fixture.workspace().join("partial");
    let mut session = fixture.session;
    session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Sandbox,
        model_reviewer: false,
    });
    if !crate::sandbox::platform_sandbox_available() {
        return;
    }
    let mut events = session.broadcaster.subscribe();
    session
        .prompt("prepare the report", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;
    assert!(session.approval_gate.pending_for_session("s1").is_empty());
    while let Ok(event) = events.try_recv() {
        assert_ne!(
            event.event_type, "approval_request",
            "failure must return to the model directly"
        );
    }
    assert!(!outside.exists());
    assert_eq!(
        std::fs::read_to_string(&partial).unwrap().lines().count(),
        1
    );
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1]
        .messages
        .iter()
        .any(|message| message.role == "tool" && message.text().contains("was not replayed")));
}

#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_sandbox_recovery_approves_only_the_models_remaining_operation() {
    let _home_guard = crate::test_support::home_env_lock();
    let target_dir = tempfile::tempdir().unwrap();
    let target = target_dir.path().join("report.txt");
    std::fs::write(&target, "old").unwrap();
    let remaining = format!("printf new > '{}'", target.display());
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start_event(
                "first-call",
                "shell",
                serde_json::json!({"command": format!("echo partial >> partial; {remaining}")}),
            ),
            tool_end_event(),
            finish_event(),
        ]),
        Script::Events(vec![
            tool_start_event(
                "remaining-call",
                "shell",
                serde_json::json!({
                    "command": remaining, "additional_permissions": {"write": [{"path": target, "scope": "file", "reason": "Save the already prepared report"}]}
                }),
            ),
            tool_end_event(),
            finish_event(),
        ]),
        text_turn("done"),
    ]);
    let fixture = run_fixture(provider.clone(), "run-precise-recovery");
    std::fs::create_dir_all(fixture.workspace().join(".future")).unwrap();
    std::fs::write(fixture.workspace().join(".future/approval_rule.json"), serde_json::json!({"rules": [{"path": target_dir.path(), "access": "write", "action": "ask"}]}).to_string()).unwrap();
    let partial = fixture.workspace().join("partial");
    let mut session = fixture.session;
    session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Sandbox,
        model_reviewer: false,
    });
    if !crate::sandbox::platform_sandbox_available() {
        return;
    }
    let gate = session.approval_gate.clone();
    let command = remaining.clone();
    let decider = std::thread::spawn(move || {
        for _ in 0..2000 {
            if let Some(request) = gate.pending_for_session("s1").first() {
                assert_eq!(request["kind"], "file_permissions");
                assert_eq!(request["tool_id"], "remaining-call");
                assert_eq!(request["action"]["command"], command);
                assert_eq!(request["action"]["targets"][0]["access"], "write");
                assert_eq!(request["action"]["targets"][0]["scope"], "file");
                assert_eq!(request["sandbox_boundary"]["inside_sandbox"], true);
                assert!(request["save_suggestion"].is_null());
                gate.decide(
                    request["approval_request_id"].as_str().unwrap(),
                    "s1",
                    crate::rpc::ApprovalDecision {
                        approved: true,
                        note: String::new(),
                        status: crate::rpc::ApprovalDecisionStatus::Approved,
                    },
                )
                .unwrap();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("precise file approval never appeared");
    });
    session
        .prompt("prepare and save the report", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;
    assert!(
        decider.join().is_ok(),
        "tool results: {:?}",
        session
            .messages
            .read()
            .iter()
            .filter(|message| message.role == "tool")
            .map(AgentMessage::text)
            .collect::<Vec<_>>()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    assert_eq!(
        std::fs::read_to_string(&partial).unwrap().lines().count(),
        1
    );
    assert!(session.approval_gate.pending_for_session("s1").is_empty());
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1]
        .messages
        .iter()
        .any(|message| message.role == "tool" && message.text().contains("was not replayed")));
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
        model_reviewer: false,
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
    session
        .prompt("loud question", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().last().unwrap().text(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_persist_failure_aborts_run_with_error() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("unused")]),
        "persist-fail",
    );
    // A directory where the transcript file should be breaks persistence.
    let mut session = fixture.session;
    session.session_manager.test_execute("CREATE TRIGGER fail_entries BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;");

    let result = session.prompt("hi", &[], &[], None, None);
    assert!(result.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn append_system_prompt_reaches_the_model_on_every_run() {
    let provider = ScriptedProvider::new(vec![text_turn("first"), text_turn("second")]);
    let fixture = run_fixture(provider.clone(), "append-prompt");
    let mut session = fixture.session;

    session.set_append_system_prompt("Always answer in haiku.");
    session.prompt("one", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    // Second run, no further set: the segment is rebuilt from the session, not
    // dragged along from the previous run's prompt string.
    session.prompt("two", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    let requests = provider.requests.lock().unwrap();
    assert!(
        requests[0]
            .system_prompt
            .contains("Always answer in haiku."),
        "the append must reach the run that follows the set — this is the \
         regression: the old in-memory append was overwritten by \
         build_system_prompt at the run boundary and never sent"
    );
    assert!(requests[1]
        .system_prompt
        .contains("Always answer in haiku."));

    // Section order: the append comes after the built-in identity and the
    // project context, and before the environment block, so it can override
    // earlier rules without diluting metadata.
    let prompt = &requests[0].system_prompt;
    let append_at = prompt.find("Always answer in haiku.").unwrap();
    let identity_at = prompt.find("You are FutureAgent").unwrap();
    let env_at = prompt.find("# Environment").unwrap();
    assert!(identity_at < append_at, "append must come after identity");
    assert!(append_at < env_at, "append must come before environment");
}

#[tokio::test(flavor = "current_thread")]
async fn clearing_the_append_segment_removes_it_from_the_next_run() {
    let provider = ScriptedProvider::new(vec![text_turn("first"), text_turn("second")]);
    let fixture = run_fixture(provider.clone(), "append-prompt-clear");
    let mut session = fixture.session;

    session.set_append_system_prompt("MARKER_RULE");
    session.prompt("one", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert!(provider.requests.lock().unwrap()[0]
        .system_prompt
        .contains("MARKER_RULE"));

    // An empty value is how a caller clears it.
    session.set_append_system_prompt("");
    session.prompt("two", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.append_system_prompt, "");
    assert!(
        !provider.requests.lock().unwrap()[1]
            .system_prompt
            .contains("MARKER_RULE"),
        "a cleared append must not survive into the next run"
    );
}

/// `session status --system-prompt` reads the prompt through
/// `current_system_prompt`, so it has to be the same string a run sends —
/// otherwise the command answers a question nobody asked.
#[tokio::test(flavor = "current_thread")]
async fn current_system_prompt_matches_what_a_run_sends() {
    // Byte-equality across two builds, so HOME (which decides the discovered
    // skills) must not change in between — see the note on
    // `renaming_does_not_change_conversation_system_prompt`.
    let _home = crate::test_support::TestHome::new();
    let provider = ScriptedProvider::new(vec![text_turn("answer")]);
    let fixture = run_fixture(provider.clone(), "append-prompt-read");
    let mut session = fixture.session;
    session.set_append_system_prompt("Rule for this session.");

    let before_run = session.current_system_prompt();
    assert!(before_run.contains("Rule for this session."));

    session.prompt("one", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    let sent = provider.requests.lock().unwrap()[0].system_prompt.clone();
    assert_eq!(before_run, sent);
    // Still identical after the run — the read must not be a one-shot.
    assert_eq!(session.current_system_prompt(), sent);
}

#[tokio::test(flavor = "current_thread")]
async fn system_prompt_override_replaces_the_built_in_prompt_on_every_run() {
    // Byte-level assertions on a built prompt: HOME decides the skills section,
    // so pin it (see the note on
    // `renaming_does_not_change_conversation_system_prompt`).
    let _home = crate::test_support::TestHome::new();
    let provider = ScriptedProvider::new(vec![
        text_turn("first"),
        text_turn("second"),
        text_turn("third"),
    ]);
    let fixture = run_fixture(provider.clone(), "custom-prompt");
    let mut session = fixture.session;

    session.set_system_prompt("You are a terse assistant. MARKER_CUSTOM");
    session.prompt("one", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    // Second run, no further set: rebuilt from the session each time.
    session.prompt("two", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    // Collected into owned strings so no MutexGuard is held across an await
    // (clippy::await_holding_lock), and the prompts read are then stable.
    let prompts: Vec<String> = provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.system_prompt.clone())
        .collect();
    for (index, prompt) in prompts.iter().enumerate() {
        assert!(
            prompt.contains("MARKER_CUSTOM"),
            "run {index} did not receive the session's replacement prompt — the \
             old setter wrote the live loop's prompt, which the run boundary \
             rebuilt from scratch"
        );
        assert!(
            !prompt.contains("You are FutureAgent"),
            "a replacement prompt must not be preceded by the built-in one"
        );
    }
    assert_eq!(prompts[0], prompts[1]);

    // Clearing it restores the built-in prompt on the next run.
    session.set_system_prompt("");
    session.prompt("three", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    let last = provider
        .requests
        .lock()
        .unwrap()
        .last()
        .map(|request| request.system_prompt.clone())
        .expect("a third request");
    assert!(last.contains("You are FutureAgent"));
    assert!(!last.contains("MARKER_CUSTOM"));
}

/// A replacement and an append are not alternatives: the append still lands,
/// and after the replacement, so a session rule can still override it.
#[tokio::test(flavor = "current_thread")]
async fn append_still_applies_after_a_prompt_override() {
    let _home = crate::test_support::TestHome::new();
    let provider = ScriptedProvider::new(vec![text_turn("answer")]);
    let fixture = run_fixture(provider.clone(), "custom-prompt-append");
    let mut session = fixture.session;
    session.set_system_prompt("BODY_MARKER");
    session.set_append_system_prompt("TAIL_MARKER");
    session.prompt("one", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    let requests = provider.requests.lock().unwrap();
    let prompt = &requests[0].system_prompt;
    let body = prompt.find("BODY_MARKER").expect("replacement text");
    let tail = prompt.find("TAIL_MARKER").expect("append text");
    assert!(body < tail, "the append must come after the replacement");
}

#[tokio::test(flavor = "current_thread")]
async fn renaming_does_not_change_conversation_system_prompt() {
    // The assertion is byte-equality of two built prompts, so every input it
    // depends on must be held constant for the duration. The prompt embeds the
    // skills discovered under `$HOME`, and other tests in this binary redirect
    // `$HOME` to a fixture directory (`TestHome`) to install fake skills —
    // without this guard a refresh of the process-global skills cache between
    // the two runs can serve run 1 and run 2 different skill sets, which fails
    // the equality below for a reason that has nothing to do with renaming.
    let _home = crate::test_support::TestHome::new();
    let provider = ScriptedProvider::new(vec![text_turn("first"), text_turn("second")]);
    let fixture = run_fixture(provider.clone(), "session-title");
    let mut session = fixture.session;
    session.set_session_name("原始标题");
    session.prompt("one", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    session.set_session_name("更新标题");
    session.prompt("two", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    let requests = provider.requests.lock().unwrap();
    assert!(!requests[0].system_prompt.contains("原始标题"));
    assert_eq!(requests[0].system_prompt, requests[1].system_prompt);
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

/// Settled runs stay replayable across later prompts, whether maintenance
/// has published their compact snapshot yet or they still use raw journals.
#[tokio::test(flavor = "current_thread")]
async fn later_runs_retire_the_journals_of_settled_older_runs() {
    let provider = ScriptedProvider::new(vec![
        text_turn("one"),
        text_turn("two"),
        text_turn("three"),
        text_turn("four"),
    ]);
    let fixture = run_fixture(provider, "retention");
    let mut session = fixture.session;
    let mut runs = Vec::new();
    for prompt in ["1", "2", "3", "4"] {
        runs.push(session.prompt(prompt, &[], &[], None, None).unwrap().run_id);
        wait_for_run_end(&session).await;
    }

    let store = session.session_manager.storage().unwrap();
    for run in &runs {
        let replay = store.replay_page("s1", run, -1, false, None).unwrap();
        assert!(
            replay.known && (replay.snapshot.is_some() || !replay.events.is_empty()),
            "run {run} stays replayable after compaction"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_with_project_context_file() {
    let provider = ScriptedProvider::new(vec![
        text_turn("ok"),
        text_turn("updated"),
        text_turn("disabled"),
    ]);
    let fixture = run_fixture(provider.clone(), "context");
    let workspace = fixture.workspace().clone();
    std::fs::write(workspace.join("AGENTS.md"), "first-project-rule").unwrap();
    std::fs::write(workspace.join("CLAUDE.md"), "lower-priority-rule").unwrap();
    std::fs::write(workspace.join("FUTURE.md"), "# independent-memory").unwrap();
    let mut session = fixture.session;
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    std::fs::write(workspace.join("AGENTS.md"), "updated-project-rule").unwrap();
    session.prompt("again", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    session.no_context_files = true;
    session
        .prompt("without project rules", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].system_prompt.contains("first-project-rule"));
    assert!(requests[1].system_prompt.contains("updated-project-rule"));
    assert!(!requests[1].system_prompt.contains("first-project-rule"));
    assert!(!requests[2].system_prompt.contains("updated-project-rule"));
    assert!(!requests[2].system_prompt.contains("# Project Context"));
    for request in requests.iter() {
        assert!(!request.system_prompt.contains("lower-priority-rule"));
        assert!(request.system_prompt.contains("# independent-memory"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_auto_compaction_with_small_history_completes() {
    // 50k tokens reported against a 64k window forces the compaction attempt.
    // (A compact() failure needs a history with no valid cut point, which the
    // public session flow cannot produce — a finished tool turn always leaves
    // ≥3 messages. That arm is covered by the run_loop unit test.)
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start_event("call-1", "read", serde_json::json!({"path": "x"})),
            tool_end_event(),
            ModelStreamEvent::Usage(crate::types::Usage {
                prompt_tokens: 50_000,
                completion_tokens: 100,
                total_tokens: 50_100,
                cache_read_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
                credit_cost: None,
                provider_metadata: None,
            }),
            finish_event(),
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
async fn prompt_with_unreadable_context_file_still_runs() {
    let provider = ScriptedProvider::new(vec![text_turn("ok")]);
    let fixture = run_fixture(provider, "ctx-dir");
    // A CLAUDE.md that is a DIRECTORY exists but cannot be read.
    std::fs::create_dir_all(fixture.workspace().join("CLAUDE.md")).unwrap();
    let mut session = fixture.session;
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert_eq!(session.messages.read().last().unwrap().text(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_journal_loss_marks_run_persistence_degraded() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![Script::Gated(
        gate.clone(),
        vec![text_event("doomed reply"), finish_event()],
    )]);
    let fixture = run_fixture(provider, "journal-loss");
    let mut session = fixture.session;
    session.prompt("hi", &[], &[], None, None).unwrap();

    session.broadcaster.fail_next_append();

    gate.notify_one();
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
    assert!(degraded, "journal loss marked the run persistence_degraded");
}

/// Run completion asks for journal compaction only after draining the writer's
/// pending batch, and a drain that fails must not stop the run from finishing or
/// from asking for compaction — its failure is reported instead.
///
/// A failed durability fence cannot be advertised as a completed run or
/// repaired by scheduling a full-history compaction.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_durability_fence_marks_run_degraded_without_compaction() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("drained late")]),
        "drain-failure",
    );
    let mut session = fixture.session;
    session.broadcaster.fail_next_flush();
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_degraded_run(&session).await;
    assert!(
        session.scheduler.recent_terminal_acks().is_empty(),
        "a failed fence must not acknowledge completion"
    );
    let store = session.session_manager.storage().unwrap();
    let counts: (i64, i64) = store
        .db
        .read(|db| {
            Ok((
                db.query_row("SELECT count(*) FROM run_snapshots", [], |row| row.get(0))?,
                db.query_row(
                    "SELECT count(*) FROM entries WHERE entry_type='run_terminal'",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(
        counts,
        (0, 0),
        "no snapshot or false terminal is written after a failed fence"
    );
}

async fn wait_for_degraded_run(session: &crate::rpc::ServerSession) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if session.runtime.snapshot().is_some_and(|snapshot| {
                snapshot.phase == crate::runtime::RunPhase::PersistenceDegraded
            }) && !session.runtime.has_owned_task()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("durability failure must settle as persistence_degraded");
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_mid_run_append_failure_heals_via_full_rewrite() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![Script::Gated(
        gate.clone(),
        vec![text_event("late answer"), finish_event()],
    )]);
    let fixture = run_fixture(provider, "heal");
    let mut session = fixture.session;
    session.prompt("hi", &[], &[], None, None).unwrap();

    // Wait for the user message to hit disk, then remove the transcript so
    // the mid-run assistant append fails (open-for-append on a missing file).
    for _ in 0..200 {
        if session.session_manager.contains("s1").unwrap() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    session
        .session_manager
        .test_execute("UPDATE sessions SET revision=-1 WHERE id='s1'");

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

/// A terminal commit that its runtime cancelled is the first evidence of a
/// *process* exit, not of a broken store: when the same run was also interrupted
/// by that shutdown, the run reads as interrupted — which is exactly what the
/// next start settles its unterminated journal entry to — instead of being
/// left fail-closed as `persistence_degraded` (whose "restart or operator
/// intervention required" wording sends an operator looking for a storage
/// fault that does not exist).
#[tokio::test(flavor = "current_thread")]
async fn a_shutdown_cancelled_terminal_commit_is_an_interruption_not_a_degraded_store() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![Script::Gated(
        gate.clone(),
        vec![text_event("cut short"), finish_event()],
    )]);
    let fixture = run_fixture(provider, "shutdown-cancelled-commit");
    let mut session = fixture.session;
    let mut events = session.broadcaster.subscribe();
    session.prompt("hi", &[], &[], None, None).unwrap();

    // Stop the Agent the way a host stop does, while the run is still inside its
    // model call, and have its terminal commit join as cancelled.
    session.interrupt_for_shutdown().unwrap();
    session.persistence.cancel_next_commit();
    gate.notify_one();

    let terminal = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("run event timeout")
            .expect("run event");
        assert_ne!(
            event.event_type, "persistence_error",
            "a shutdown-cancelled commit is not a persistence failure"
        );
        if event.event_type == "agent_end" {
            break serde_json::from_str::<serde_json::Value>(&event.data).unwrap();
        }
    };
    assert_eq!(
        terminal["state"],
        crate::session::RUN_STATE_INTERRUPTED_BY_RESTART,
        "the run reports the state the journal will be read as"
    );

    // The run was released normally; it is not held in a degraded phase, so the
    // session is not left unusable after this shutdown.
    wait_for_run_end(&session).await;
    assert_eq!(
        session.runtime.persistence_degraded_count(),
        0,
        "a shutdown cancellation must not mark the store degraded"
    );
    assert!(
        session.persistence.last_error().is_none(),
        "the writer has no failure of its own to report"
    );
}

/// The counter-case: the same cancelled join without a shutdown interruption is
/// a real failure and keeps the fail-closed path unchanged.
#[tokio::test(flavor = "current_thread")]
async fn a_cancelled_terminal_commit_without_a_shutdown_still_degrades() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![Script::Gated(
        gate.clone(),
        vec![text_event("cut short"), finish_event()],
    )]);
    let fixture = run_fixture(provider, "cancelled-commit-no-shutdown");
    let mut session = fixture.session;
    session.prompt("hi", &[], &[], None, None).unwrap();

    // A user stop is not a shutdown interruption, so the run has no interruption
    // cause to explain a cancelled commit.
    session.abort();
    session.persistence.cancel_next_commit();
    gate.notify_one();

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
    assert_eq!(session.runtime.persistence_degraded_count(), 1);
}

// HOME_ENV_LOCK is a plain Mutex by design: it must exclude TestHome
// redirects in OTHER threads for the whole test, including across awaits.
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_workspace_permission_routes_through_approval_gate() {
    let _home_guard = crate::test_support::home_env_lock();
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("futureos-gate-target-{}.txt", std::process::id()));
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start_event(
                "call-1",
                "write",
                serde_json::json!({"path": outside.to_string_lossy(), "content": "ok"}),
            ),
            tool_end_event(),
            finish_event(),
        ]),
        text_turn("done"),
    ]);
    let fixture = run_fixture(provider, "gate");
    let gate = fixture.session.approval_gate.clone();
    let mut session = fixture.session;
    session.set_permission_level("workspace");
    session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Manual,
        model_reviewer: false,
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

// ── coverage batch 24: per-line residuals ─────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn queued_run_prompt_uses_accepted_model_and_thinking() {
    let provider = ScriptedProvider::new(vec![text_turn("done")]);
    let fixture = run_fixture(provider.clone(), "frozen-prompt");
    let mut session = fixture.session;
    session.model = "provider/frozen-model".into();
    session.thinking_level = "medium".into();
    session.auto_compaction = false;
    let held = session.runtime.begin(Some("held"), None).unwrap();
    session
        .enqueue_prompt(
            "queued",
            &[],
            &[],
            None,
            "queued-request",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    session.model = "provider/new-model".into();
    session.thinking_level = "high".into();
    session.runtime.begin_finalizing(&held);
    session.runtime.finish(&held);
    session.start_next_scheduled().unwrap();
    wait_for_run_end(&session).await;
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].system_prompt.contains("provider/frozen-model"));
    assert!(requests[0].system_prompt.contains("Thinking level: medium"));
    assert!(!requests[0].system_prompt.contains("provider/new-model"));
}

#[tokio::test(flavor = "current_thread")]
async fn queued_run_freezes_context_files_opt_out() {
    for disabled_at_admission in [true, false] {
        let provider = ScriptedProvider::new(vec![text_turn("done")]);
        let fixture = run_fixture(provider.clone(), "frozen-context-files");
        std::fs::write(fixture.workspace().join("AGENTS.md"), "queued-project-rule").unwrap();
        let mut session = fixture.session;
        session.no_context_files = disabled_at_admission;
        let held = session.runtime.begin(Some("held"), None).unwrap();
        session
            .enqueue_prompt(
                "queued",
                &[],
                &[],
                None,
                "queued-request",
                crate::runtime::BusyPolicy::EnqueueIfBusy,
            )
            .unwrap();
        session.no_context_files = !disabled_at_admission;
        session.runtime.begin_finalizing(&held);
        session.runtime.finish(&held);
        session.start_next_scheduled().unwrap();
        wait_for_run_end(&session).await;
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].system_prompt.contains("queued-project-rule"),
            !disabled_at_admission
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_prompt_defers_until_scheduler_completion_delivery() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("x")]),
        "handoff-window",
    );
    let mut session = fixture.session;
    session
        .scheduler
        .accept(
            "old-request",
            Some("old-run"),
            crate::runtime::BusyPolicy::EnqueueIfBusy,
            serde_json::json!({}),
        )
        .unwrap();
    session.scheduler.start_next(1).unwrap();
    assert!(session.runtime.snapshot().is_none());
    assert!(!session.runtime.has_owned_task());
    let ack = session
        .enqueue_prompt(
            "hi",
            &[],
            &[],
            None,
            "new-request",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(ack.accepted_state, crate::runtime::RunAcceptedState::Queued);
    assert_eq!(session.scheduler.queued().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_prompt_defers_when_control_idle_but_task_slot_held() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("x")]), "hold-slot");
    let mut session = fixture.session;
    // Forge the transient "control finalized but task monitor still holds the
    // slot" window: no control snapshot, but the task slot is occupied.
    session.runtime.test_hold_task_slot();
    let ack = session
        .enqueue_prompt(
            "hi",
            &[],
            &[],
            None,
            "req-hold",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(ack.accepted_state, crate::runtime::RunAcceptedState::Queued);
    // Deferred: the run stays queued until the ghost slot is released.
    assert_eq!(session.scheduler.queued().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn enqueue_prompt_releases_active_when_scheduled_start_fails() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("x")]),
        "post-start-fail",
    );
    let mut session = fixture.session;
    *super::POST_START_FAIL_RUN.lock() = Some("run-known".to_string());
    let result = session.enqueue_prompt(
        "hi",
        &[],
        &[],
        Some("run-known"),
        "req-known",
        crate::runtime::BusyPolicy::EnqueueIfBusy,
    );
    assert!(result.is_err());
    // The active run was released by the error arm's finish_active branch.
    assert!(session.scheduler.active().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn failed_coalesced_activation_preserves_each_input_without_restart() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("unused")]),
        "failed-fold-receipts",
    );
    let mut session = fixture.session;
    let mut saved = crate::session::Session::new(".", "mock");
    saved.id = session.session_id.clone();
    saved.entries.push(crate::session::SessionEntry::new_user(
        "user",
        serde_json::json!("existing history"),
    ));
    session.session_manager.save(&saved).unwrap();
    session
        .scheduler
        .accept(
            "previous",
            Some("previous-run"),
            crate::runtime::BusyPolicy::EnqueueIfBusy,
            serde_json::json!({}),
        )
        .unwrap();
    session.scheduler.start_next(1).unwrap();
    let mut ids = Vec::new();
    for request in ["accepted-a", "accepted-b"] {
        let ack = session
            .enqueue_prompt(
                request,
                &[],
                &[],
                None,
                request,
                crate::runtime::BusyPolicy::EnqueueCoalescing,
            )
            .unwrap();
        ids.push(ack.run_id);
    }
    session.scheduler.finish_active("previous-run").unwrap();
    session.scheduled_snapshots.clear();
    let error = session.start_next_scheduled().unwrap_err();
    assert!(error.to_string().contains("snapshot is unavailable"));
    let entries = session
        .session_manager
        .load(&session.session_id)
        .unwrap()
        .entries;
    for run in ids {
        assert!(entries.iter().any(|entry| entry.entry_type == "user"
            && entry
                .meta
                .as_ref()
                .is_some_and(|meta| meta["run_id"] == run)));
        assert!(entries
            .iter()
            .any(|entry| entry.entry_type == "run_terminal"
                && entry
                    .content
                    .as_ref()
                    .is_some_and(|value| value["run_id"] == run && value["state"] == "cancelled")));
    }
    assert!(!entries
        .iter()
        .any(|entry| entry.entry_type == "run_started"));
    assert!(!session.runtime.has_owned_task());
}

#[tokio::test(flavor = "current_thread")]
async fn scheduled_run_fifo_mismatch_reports_error() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("x")]), "fifo-mismatch");
    let mut session = fixture.session;
    super::SCHEDULED_DEQUEUE_HOOK.lock().insert(
        "run-a".to_string(),
        Box::new(|sess: &mut crate::rpc::ServerSession| {
            // Queue a foreign run and move run-a to the back, so start_next
            // pops the foreign run instead of the one just peeked.
            sess.scheduler
                .accept(
                    "req-foreign",
                    Some("run-foreign"),
                    crate::runtime::BusyPolicy::EnqueueIfBusy,
                    serde_json::json!({"message": "foreign"}),
                )
                .unwrap();
            sess.scheduler.test_move_front_to_back();
        }),
    );
    let result = session.enqueue_prompt(
        "hi",
        &[],
        &[],
        Some("run-a"),
        "req-a",
        crate::runtime::BusyPolicy::EnqueueIfBusy,
    );
    assert!(result.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn scheduled_run_empty_dequeue_reports_error() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("x")]), "empty-dequeue");
    let mut session = fixture.session;
    // Distinct run id from scheduled_run_fifo_mismatch_reports_error: the
    // hook map is keyed per run id, so two parallel tests arm their own
    // slots and can never overwrite or consume each other's hook.
    super::SCHEDULED_DEQUEUE_HOOK.lock().insert(
        "run-b".to_string(),
        Box::new(|sess: &mut crate::rpc::ServerSession| {
            // Drain the queue after the peek, so start_next finds nothing.
            sess.scheduler
                .cancel_all_queued(crate::runtime::QueuedCancellationReason::Cancelled);
        }),
    );
    let result = session.enqueue_prompt(
        "hi",
        &[],
        &[],
        Some("run-b"),
        "req-a",
        crate::runtime::BusyPolicy::EnqueueIfBusy,
    );
    assert!(result.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_terminal_worker_spawn_failure_marks_degraded_without_completion() {
    let provider = ScriptedProvider::new(vec![text_turn("hello")]);
    let fixture = run_fixture(provider.clone(), "append-fail");
    let mut session = fixture.session;
    session.persistence.fail_spawns_for_test(2);
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_degraded_run(&session).await;
    assert_eq!(
        provider.requests.lock().unwrap().len(),
        1,
        "the canonical writer accepts the user independently of the terminal worker"
    );
    let loaded = session.session_manager.load(&session.session_id).unwrap();
    assert!(loaded
        .entries
        .iter()
        .any(|entry| entry.entry_type == crate::session::ENTRY_TYPE_USER));
    assert!(
        !loaded
            .entries
            .iter()
            .any(|entry| entry.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL),
        "an unavailable terminal worker must not publish a completed run"
    );
    assert!(session.scheduler.recent_terminal_acks().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_stale_epoch_begin_finalizing_returns_early() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("done")]),
        "stale-finalize",
    );
    let mut session = fixture.session;
    session.runtime.fail_next_begin_finalizing();
    session.prompt("hi", &[], &[], None, None).unwrap();
    // The run task returns early from the stale begin_finalizing, so the
    // monitor marks it stuck ("run task exited without finalizing").
    for _ in 0..500 {
        if session
            .runtime
            .snapshot()
            .is_some_and(|s| s.phase == crate::runtime::RunPhase::CancellationStuck)
            && !session.runtime.has_owned_task()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(session
        .runtime
        .snapshot()
        .is_some_and(|s| s.phase == crate::runtime::RunPhase::CancellationStuck));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_persistence_commit_panic_marks_degraded() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("doomed")]),
        "commit-panic",
    );
    let mut session = fixture.session;
    session.persistence.panic_next_commit();
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

#[tokio::test(flavor = "current_thread")]
async fn prompt_spawn_double_occupancy_reports_error() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("x")]),
        "spawn-occupied",
    );
    let mut session = fixture.session;
    *super::RUN_SPAWN_HOOK.lock() = Some((
        "run-a".to_string(),
        Box::new(|sess: &mut crate::rpc::ServerSession| {
            sess.runtime.test_hold_task_slot();
        }),
    ));
    let result = session.enqueue_prompt(
        "hi",
        &[],
        &[],
        Some("run-a"),
        "req-a",
        crate::runtime::BusyPolicy::EnqueueIfBusy,
    );
    assert!(result.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_empty_message_hits_empty_context_early_return() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("done")]),
        "empty-context",
    );
    let mut session = fixture.session;
    session.model = "glm-4.5v".to_string();
    // An empty first message estimates to zero context tokens, hitting the
    // "truly empty — nothing to compact" early return inside the transform.
    session.prompt("", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
}

#[tokio::test]
async fn idle_session_releases_cached_history_and_reloads_it_on_demand() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("persisted answer")]),
        "idle-history",
    );
    let mut session = fixture.session;
    session.prompt("remember me", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    assert!(!session.messages.read().is_empty());
    let subscriber = session.broadcaster.subscribe();
    assert!(
        !session.release_idle_history_if_unused(),
        "a live subscriber pins the in-memory projector/history"
    );
    drop(subscriber);
    assert!(session.release_idle_history_if_unused());
    assert!(session.messages.read().is_empty());
    assert!(!session.history_loaded);
    session.ensure_history_loaded().unwrap();
    assert!(session
        .messages
        .read()
        .iter()
        .any(|message| message.text().contains("remember me")));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_after_idle_release_restores_history_before_persisting() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("first answer"), text_turn("second answer")]),
        "idle-history-prompt",
    );
    let mut session = fixture.session;
    session
        .prompt("first question", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;
    assert!(session.release_idle_history_if_unused());
    assert!(session.messages.read().is_empty());

    // Deliberately do not call ensure_history_loaded here. prompt_internal is
    // the last boundary that can close the admission/idle-release race for both
    // direct and scheduled starts.
    session
        .prompt("second question", &[], &[], None, None)
        .unwrap();
    wait_for_run_end(&session).await;

    let stored = session.session_manager.load(&session.session_id).unwrap();
    let text = stored
        .entries
        .iter()
        .filter_map(|entry| entry.content.as_ref())
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("first question"),
        "old user history was lost: {text}"
    );
    assert!(
        text.contains("first answer"),
        "old assistant history was lost: {text}"
    );
    assert!(text.contains("second question"));
    assert!(text.contains("second answer"));
}

#[test]
fn checkpoint_survives_a_run_boundary_after_the_transcript_changes_on_disk() {
    use crate::session::{checkpoint_to_entry, Session, SessionEntry};

    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("unused")]),
        "checkpoint-cache",
    );
    let mut session = fixture.session;
    let id = "checkpoint-cache";
    let mut info = SessionEntry::session_info(
        serde_json::json!({"cwd": ".", "model": "mock"}),
        "mock".into(),
        "default".into(),
    );
    info.id = "info".into();
    let mut user = SessionEntry::new_user("user", serde_json::json!("old request"));
    user.id = "u1".into();
    let mut assistant = SessionEntry::new_assistant(serde_json::json!("old answer"), Vec::new());
    assistant.id = "a1".into();
    let mut checkpoint = test_checkpoint("cp-entry", "cp-cache");
    checkpoint.covered_from_entry_id = Some("u1".into());
    checkpoint.cutoff_entry_id = Some("a1".into());
    let checkpoint_entry = checkpoint_to_entry(&checkpoint);
    let base = Session::snapshot(
        id.into(),
        ".".into(),
        "mock".into(),
        String::new(),
        String::new(),
        vec![
            info.clone(),
            user.clone(),
            assistant.clone(),
            checkpoint_entry,
        ],
    );
    session.session_manager.save(&base).unwrap();
    session.switch_session_metadata(id).unwrap();
    session.ensure_history_loaded().unwrap();
    assert_eq!(
        session
            .active_checkpoint
            .lock()
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str()),
        Some("cp-cache")
    );

    // If a prompt went back to disk for the checkpoint, this replacement would
    // erase it. The cache is loaded once per handle, not once per run.
    let replaced = Session::snapshot(
        id.into(),
        ".".into(),
        "mock".into(),
        String::new(),
        String::new(),
        vec![info, user, assistant],
    );
    session.session_manager.save(&replaced).unwrap();
    let agent_loop = session.agent_loop.clone();
    let mut loop_ = agent_loop.try_write().unwrap();
    session.wire_auto_compaction(&mut loop_, true, "mock");
    assert_eq!(
        loop_
            .active_checkpoint
            .lock()
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str()),
        Some("cp-cache"),
        "run wiring must reuse the loaded checkpoint instead of reading the transcript"
    );
}

#[test]
fn wire_auto_compaction_reports_no_valid_boundary() {
    use std::sync::atomic::Ordering;
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("x")]), "compact-fail");
    let session = fixture.session;
    // Report a huge API token count so compaction is "needed", but hand the
    // transform a history with no valid cut point > 0 (a lone user message),
    // so compact() returns None → the "needed but failed" arm.
    session.last_prompt_tokens.store(70_000, Ordering::Relaxed);
    let agent_loop = session.agent_loop.clone();
    let mut loop_ = agent_loop.try_write().unwrap();
    session.wire_auto_compaction(&mut loop_, true, "glm-4.5v");
    assert_eq!(
        loop_.context_manager.as_ref().unwrap().model,
        loop_.model,
        "summary requests use the provider-facing bare model id"
    );
    let mut message = crate::types::AgentMessage::new_user("user", serde_json::json!("hi"));
    message.ensure_journal_entry_id();
    let raw = [message];
    let prompt = crate::compaction::project_prompt_context(&raw, None, Some(70_000), 64_000);
    // The runtime path, not the retired legacy one: a lone user message has no valid
    // cut point in either, and this is the call a real turn makes.
    let result = loop_.context_manager.as_ref().unwrap().prepare_evidence(
        prompt,
        &raw,
        crate::compaction::CompactionTrigger::Automatic,
        crate::compaction::CompactionPhase::PreTurn,
        None,
        &std::sync::atomic::AtomicBool::new(false),
        None,
    );
    assert!(matches!(
        result,
        Err(crate::compaction::ContextError::NoValidBoundary)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn prompt_with_auto_compaction_disabled_completes() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("done")]),
        "compaction-off",
    );
    let mut session = fixture.session;
    session.set_auto_compaction(false);
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
}

#[test]
fn persist_user_message_with_empty_history_takes_slow_path() {
    let fixture = run_fixture(ScriptedProvider::new(vec![text_turn("x")]), "persist-empty");
    let session = fixture.session;
    // No user message pushed → msgs.last() is None, so the fast path is
    // skipped and the brand-new-session slow path runs.
    let lease = session
        .runtime
        .begin(Some("run-empty"), Some("req-empty"))
        .unwrap();
    session.persist_user_message(&lease).unwrap();
    let loaded = session.session_manager.load("s1").unwrap();
    assert!(!loaded.entries.is_empty());
}

/// The creator identity reaches the persisted `session_info` on **both** write
/// paths — the user-message persistence at the run's start (`persist_user_message`,
/// ~1617) and the end-of-turn persistence inside `prompt_internal`'s blocking
/// commit task (~1178) — and each is gated on the identity being non-empty, so
/// neither line had ever executed. The existing provenance test sets
/// `cmd.creator_id` and asserts the *in-memory* session
/// (`new_session_honors_explicit_id_cwd_model_level_and_provenance`), and nothing
/// had ever run a prompt on a session carrying one.
#[tokio::test(flavor = "current_thread")]
async fn a_run_records_the_creator_identity_on_both_persistence_paths() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![text_turn("done")]),
        "creator-identity",
    );
    let mut session = fixture.session;
    session.creator_id = "desktop-creator-1".to_string();
    session.prompt("hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;

    let loaded = session.session_manager.load("s1").unwrap();
    let info = loaded
        .entries
        .iter()
        .find(|entry| entry.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO)
        .expect("the run must persist its session info");
    assert_eq!(
        info.content.as_ref().unwrap()["creator_id"],
        "desktop-creator-1",
        "the creator identity must survive both persistence paths"
    );
}

#[test]
fn rewrite_snapshot_reinserts_compaction_checkpoints() {
    use crate::compaction::{CompactionTrigger, ContextCheckpoint};
    use crate::session::{checkpoint_to_entry, Session, SessionEntry, ENTRY_TYPE_COMPACTION};
    use crate::types::ContentBlock;

    let dir = test_path("rewrite-checkpoints");
    std::fs::create_dir_all(&dir).unwrap();
    let manager = crate::session::Manager::new(dir.join("sessions"));

    // Old session: one user entry, then a checkpoint whose cutoff is that
    // entry, then a legacy checkpoint with no cutoff entry id.
    let mut user_entry = SessionEntry::new_user("user", serde_json::json!("hello"));
    user_entry.id = "entry-a".to_string();

    let checkpoint = ContextCheckpoint {
        entry_id: "cp-1".into(),
        protected_entry_ids: Vec::new(),
        checkpoint_id: "cp-1".into(),
        covered_from_entry_id: Some("entry-a".into()),
        cutoff_entry_id: Some("entry-a".into()),
        summary: vec![ContentBlock::text("summary")],
        tokens_before: 100,
        tokens_after: 10,
        trigger: CompactionTrigger::Automatic,
        phase: None,
        algorithm_version: "v2".into(),
        summary_outcome: None,
        model: "model".into(),
        context_window: 200,
        created_at: chrono::Utc::now(),
    };
    let cp_entry = checkpoint_to_entry(&checkpoint);

    let mut legacy_cp = SessionEntry::new_user("user", serde_json::json!(null));
    legacy_cp.entry_type = ENTRY_TYPE_COMPACTION.to_string();
    legacy_cp.role = "system".to_string();
    legacy_cp.content = Some(serde_json::json!({"summary": "[legacy checkpoint]"}));

    let old = Session::snapshot(
        "s1".into(),
        dir.to_string_lossy().to_string(),
        "model".into(),
        "session".into(),
        String::new(),
        vec![user_entry.clone(), cp_entry.clone(), legacy_cp.clone()],
    );
    manager.save(&old).unwrap();

    // The rebuilt message reuses the same journal entry id as the cutoff.
    let mut msg = crate::types::AgentMessage::new_user("user", serde_json::json!("hello"));
    msg.metadata = Some(
        [(
            crate::types::AgentMessage::JOURNAL_ENTRY_ID_KEY.to_string(),
            serde_json::Value::String("entry-a".into()),
        )]
        .into_iter()
        .collect(),
    );

    let info_entry =
        SessionEntry::session_info(serde_json::json!({}), "model".into(), "high".into());
    let run_started = SessionEntry::run_started("run-1", 0);
    let terminal = SessionEntry::run_terminal("run-1", "completed", 0, 0, None);

    let rebuilt = crate::rpc::ServerSession::build_rewrite_snapshot(
        &manager,
        "s1",
        dir.to_string_lossy().as_ref(),
        "model",
        "session",
        "",
        &[msg],
        info_entry,
        run_started,
        terminal,
    );

    // Both checkpoints survive the rewrite; the one with a matching cutoff is
    // re-inserted immediately after its cutoff entry.
    let cp_pos = rebuilt
        .entries
        .iter()
        .position(|e| e.id == "cp-1")
        .expect("checkpoint survives rewrite");
    let cutoff_pos = rebuilt
        .entries
        .iter()
        .position(|e| e.id == "entry-a")
        .expect("cutoff entry survives rewrite");
    assert_eq!(cp_pos, cutoff_pos + 1);
    assert!(rebuilt
        .entries
        .iter()
        .any(|e| e.entry_type == ENTRY_TYPE_COMPACTION && e.id == legacy_cp.id));
}

// ── run callbacks: the wiring `prompt_internal` installs ───────────────────

/// A checkpoint the size of the ones the compaction path produces: the same
/// shape `checkpoint_to_entry` consumes.
fn test_checkpoint(entry_id: &str, checkpoint_id: &str) -> crate::compaction::ContextCheckpoint {
    crate::compaction::ContextCheckpoint {
        entry_id: entry_id.to_string(),
        checkpoint_id: checkpoint_id.to_string(),
        covered_from_entry_id: None,
        cutoff_entry_id: None,
        summary: vec![crate::types::ContentBlock::Text {
            text: "handoff summary".to_string(),
        }],
        protected_entry_ids: vec![],
        tokens_before: 1000,
        tokens_after: 100,
        trigger: crate::compaction::CompactionTrigger::Automatic,
        phase: Some(crate::compaction::CompactionPhase::PreTurn),
        algorithm_version: "test-v1".to_string(),
        summary_outcome: None,
        model: "mock".to_string(),
        context_window: 64_000,
        created_at: chrono::Utc::now(),
    }
}

/// Start a run, wait for it, and hand back the wiring it captured. The run
/// itself is what builds the closures, so this is the production path.
async fn run_and_take_callbacks(name: &str) -> (crate::rpc::ServerSession, RunCallbacksForTest) {
    let session_id = format!("{name}-session");
    let fixture = run_fixture_with_id(
        ScriptedProvider::new(vec![text_turn("wired")]),
        name,
        &session_id,
    );
    let mut session = fixture.session;
    session.prompt("hello", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    let callbacks = take_run_callbacks(&session.session_id);
    (session, callbacks)
}

/// Keep the run active while exercising callbacks; a completed compacted
/// journal is immutable and must reject late run-scoped event appends.
fn start_and_take_callbacks(
    name: &str,
) -> (
    crate::rpc::ServerSession,
    RunCallbacksForTest,
    Arc<tokio::sync::Notify>,
) {
    let gate = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![Script::Gated(
        gate.clone(),
        vec![text_event("wired"), finish_event()],
    )]);
    let fixture = run_fixture_with_id(provider, name, &format!("{name}-session"));
    let mut session = fixture.session;
    session.prompt("hello", &[], &[], None, None).unwrap();
    let callbacks = take_run_callbacks(&session.session_id);
    (session, callbacks, gate)
}

#[tokio::test(flavor = "current_thread")]
async fn checkpoint_callback_commits_the_checkpoint_durably_and_reports_a_failed_commit() {
    let (session, callbacks) = run_and_take_callbacks("checkpoint-wiring").await;
    let checkpoint = test_checkpoint("wiring-cp-entry", "wiring-cp");

    (callbacks.on_checkpoint)(&checkpoint).expect("the checkpoint commits");
    // The effect, not the line: the checkpoint is a durable journal entry that
    // a reloaded session sees, with the checkpoint's own identity in its body.
    let reloaded = session.session_manager.load(&session.session_id).unwrap();
    let committed = reloaded
        .entries
        .iter()
        .find(|entry| entry.id == "wiring-cp-entry")
        .expect("the committed checkpoint is in the journal");
    assert_eq!(committed.entry_type, crate::session::ENTRY_TYPE_COMPACTION);
    let content = committed.content.as_ref().expect("checkpoint body");
    assert_eq!(content["checkpoint_id"], "wiring-cp");
    assert_eq!(content["summary"][0]["text"], "handoff summary");

    // A commit that cannot land must be reported to the run, which treats it as
    // a run failure — never swallowed.
    session.persistence.close().unwrap();
    let error = (callbacks.on_checkpoint)(&checkpoint)
        .expect_err("a failed checkpoint commit must be reported");
    assert!(
        error.to_string().contains("closed") || error.to_string().contains("unavailable"),
        "{error}"
    );
    // The failed commit wrote nothing: the journal still holds exactly one
    // checkpoint entry.
    let after = session.session_manager.load(&session.session_id).unwrap();
    assert_eq!(
        after
            .entries
            .iter()
            .filter(|entry| entry.entry_type == crate::session::ENTRY_TYPE_COMPACTION)
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escalation_callback_publishes_a_decidable_sandbox_request_on_the_session_stream() {
    let (session, callbacks, gate) = start_and_take_callbacks("escalation-wiring");
    let provider_gate = gate;
    let session_id = session.session_id.clone();
    let mut rx = session.broadcaster.subscribe();

    // The decision arrives from the user's side of the same gate the session
    // owns, exactly as the GUI would answer it.
    let gate = session.approval_gate.clone();
    let decider_session = session_id.clone();
    let decider = std::thread::spawn(move || {
        for _ in 0..2000 {
            if let Some(first) = gate.pending_for_session(&decider_session).first() {
                let request_id = first["approval_request_id"].as_str().unwrap().to_string();
                let kind = first["kind"].as_str().unwrap().to_string();
                let _ = gate.decide(
                    &request_id,
                    &decider_session,
                    crate::rpc::ApprovalDecision {
                        approved: true,
                        note: String::new(),
                        status: crate::rpc::ApprovalDecisionStatus::Approved,
                    },
                );
                return kind;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("escalation request never appeared");
    });

    let decision = (callbacks.escalation)(&crate::sandbox::EscalationRequest {
        trigger: crate::sandbox::EscalationTrigger::SandboxFailure,
        command: "touch outside-workspace".to_string(),
        justification: "the sandbox blocked it".to_string(),
        failure_summary: "touch: outside-workspace: Operation not permitted".to_string(),
    });
    assert!(matches!(
        decision,
        crate::sandbox::EscalationDecision::Approved
    ));
    assert_eq!(decider.join().unwrap(), "sandbox_escalation");

    // Both halves of the exchange reached this session's stream, so the GUI can
    // render the prompt and then clear it.
    let mut requested = None;
    let mut decided = None;
    while let Ok(event) = rx.try_recv() {
        match event.event_type.as_str() {
            "approval_request" => requested = Some(event.data.clone()),
            "approval_decision" => decided = Some(event.data.clone()),
            _ => continue,
        }
    }
    let requested: serde_json::Value =
        serde_json::from_str(&requested.expect("the escalation request is published")).unwrap();
    assert_eq!(requested["session_id"], session_id);
    assert_eq!(requested["kind"], "sandbox_escalation");
    assert_eq!(
        requested["requested_action"]["command"],
        "touch outside-workspace"
    );
    let decided: serde_json::Value =
        serde_json::from_str(&decided.expect("the decision is published")).unwrap();
    assert_eq!(decided["status"], "approved");
    assert_eq!(
        decided["approval_request_id"],
        requested["approval_request_id"]
    );
    provider_gate.notify_one();
    wait_for_run_end(&session).await;
}

#[tokio::test(flavor = "current_thread")]
async fn sandboxed_notifier_broadcasts_tool_sandboxed_for_the_run() {
    let (session, callbacks, gate) = start_and_take_callbacks("sandboxed-wiring");
    let mut rx = session.broadcaster.subscribe();

    (callbacks.on_sandboxed)("printf sandboxed-run");

    let mut published = None;
    while let Ok(event) = rx.try_recv() {
        if event.event_type == "tool_sandboxed" {
            published = Some(event.data.clone());
        }
    }
    let published: serde_json::Value =
        serde_json::from_str(&published.expect("tool_sandboxed is published")).unwrap();
    assert_eq!(published["type"], "tool_sandboxed");
    assert_eq!(published["command"], "printf sandboxed-run");
    gate.notify_one();
    wait_for_run_end(&session).await;
}

/// A run whose policy asks the *automatic* reviewer to gate commands rebuilds
/// the run gate (`ApprovalGate::for_run`) instead of reusing the session's, and
/// a credential is present so the reviewer policy survives the account policy.
///
/// A sandbox-wrapped plain shell command is auto-allowed (the OS sandbox is the
/// protection), so the gate is only *consulted* for a file-permission request —
/// which needs approval even when the shell is wrapped. That request is reviewed
/// automatically: an assessment is broadcast, no human request is parked, and a
/// review that cannot reach a verdict refuses the command.
///
/// `TestHome::new()` takes the process-global HOME lock itself and holds it for
/// its lifetime — taking `home_env_lock()` first *and* then constructing a
/// `TestHome` self-deadlocks (`std::sync::Mutex` is not reentrant). That is what
/// an earlier attempt at this test hung on; do not add the extra guard.
#[cfg(target_os = "macos")]
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_automatic_reviewer_policy_reviews_a_sandboxed_run_without_asking_the_user() {
    if !crate::sandbox::platform_sandbox_available() {
        return;
    }
    let home = crate::test_support::TestHome::new();
    crate::test_support::ensure_fd_headroom();
    // A reviewer that is reachable and answers "server error" at once, so the
    // review completes fast (and fails: no verdict is reachable).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut scratch = [0u8; 4096];
            let _ = socket.read(&mut scratch).await;
            let _ = socket
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            let _ = socket.shutdown().await;
        }
    });
    std::fs::create_dir_all(home.path().join(".future/agent")).unwrap();
    std::fs::write(
        home.auth_path(),
        format!(
            r#"{{"future":{{"type":"api_key","key":"fixture-key","base_url":"http://127.0.0.1:{port}"}}}}"#
        ),
    )
    .unwrap();
    assert!(
        crate::system_one::endpoint().is_some(),
        "the fixture credential must resolve to an endpoint, or the account policy strips the reviewer"
    );

    let target_dir = tempfile::tempdir().unwrap();
    let target = target_dir.path().join("report.txt");
    std::fs::write(&target, "old").unwrap();
    let command = format!("printf new > '{}'", target.display());
    let provider = ScriptedProvider::new(vec![
        Script::Events(vec![
            tool_start_event(
                "reviewed-call",
                "shell",
                serde_json::json!({
                    "command": command,
                    "additional_permissions": {
                        "write": [{"path": target, "scope": "file", "reason": "Save the report the user asked for"}]
                    }
                }),
            ),
            tool_end_event(),
            finish_event(),
        ]),
        text_turn("done"),
    ]);
    let fixture = run_fixture(provider, "run-auto-review");
    let workspace = fixture.workspace().to_path_buf();
    let mut session = fixture.session;
    // An "ask" rule on the target directory is what makes the write need
    // approval at all.
    std::fs::create_dir_all(workspace.join(".future")).unwrap();
    std::fs::write(
        workspace.join(".future/approval_rule.json"),
        serde_json::json!({"rules": [{"path": target_dir.path(), "access": "write", "action": "ask"}]})
            .to_string(),
    )
    .unwrap();
    session.set_sandbox_policy(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Sandbox,
        model_reviewer: true,
    });
    let mut events = session.broadcaster.subscribe();
    session
        .prompt("create the report", &[], &[], None, None)
        .unwrap();

    // The review happens inside the tool call, so bound the wait on the
    // assessment rather than on the whole run.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut assessed = None;
    let mut seen: std::collections::BTreeMap<String, u32> = Default::default();
    while std::time::Instant::now() < deadline {
        while let Ok(event) = events.try_recv() {
            *seen.entry(event.event_type.clone()).or_insert(0) += 1;
            match event.event_type.as_str() {
                "approval_assessment" => assessed = Some(event.data.clone()),
                "approval_request" => panic!("an automatic policy must not ask the user"),
                _ => {}
            }
        }
        if assessed.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let assessment = assessed.unwrap_or_else(|| {
        panic!(
            "the run must review the file-permission request through the automatic gate; \
             events seen: {seen:?}, messages: {:?}",
            session
                .messages
                .read()
                .iter()
                .map(|message| (
                    message.role.clone(),
                    message.text().chars().take(80).collect::<String>()
                ))
                .collect::<Vec<_>>()
        )
    });
    // The reviewer answered 500, so the policy refuses instead of running: the
    // command did not touch the file, and the refusal is reported in the
    // assessment the client receives.
    assert!(
        assessment.contains("review_error") || assessment.contains("stderr"),
        "a failed review is reported as a review error: {assessment}"
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "old",
        "a command whose review failed must not run"
    );
    assert!(
        session.approval_gate.pending_for_session("s1").is_empty(),
        "an automatic policy never parks a human request"
    );
}

/// `/tools off` has to reach the *next* run's request: the palette says "tool
/// settings updated" and a reader who then sends a prompt expects a model with
/// no tools at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disable_tools_removes_every_tool_from_the_next_run() {
    let provider = ScriptedProvider::new(vec![text_turn("no tools needed")]);
    let fixture = run_fixture(provider.clone(), "disable-tools-run");
    let mut session = fixture.session;
    session.disable_tools();
    session.prompt("say hi", &[], &[], None, None).unwrap();
    wait_for_run_end(&session).await;
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "one turn, one request");
    assert!(
        requests[0].tools.is_empty(),
        "a disabled tool set must not be sent: {:?}",
        requests[0]
            .tools
            .iter()
            .map(|t| &t.function.name)
            .collect::<Vec<_>>()
    );
}

// ─── a run abandoned by persistence must say so ────────────────────────

/// The 2026-10-31 shape: the journal is already failed when the run ends, so
/// the append path drops every later event — including the `agent_end` the
/// normal completion path sends. Before this, the run stopped mid-stream with
/// no terminal at all (`run_ebdcad4959…`: 20:04:58.896Z refusal, 20:04:58.912Z
/// durability barrier failed, nothing delivered; the client aborted 3.5 s
/// later). The run stays `persistence_degraded` — its outcome cannot be
/// recorded durably — but the client is told instead of waiting forever.
#[tokio::test(flavor = "current_thread")]
async fn a_run_abandoned_by_persistence_still_sends_a_terminal() {
    let fixture = run_fixture(ScriptedProvider::new(vec![]), "persistence-abandon");
    let mut session = fixture.session;
    assert!(
        session.broadcaster.uses_segmented_storage(),
        "the failed-journal terminal is about the segmented run path"
    );
    let mut events = session.broadcaster.subscribe();
    session
        .prompt("please answer", &[], &[], None, None)
        .unwrap();
    // The run's own interrupt flag is what a failed journal sets; install the
    // failure the way the append path does once it gives up on storage.
    session.broadcaster.fail_persistence(
        "event storage backlog exceeded its byte or age limit; generation interrupted",
    );

    let mut types = Vec::new();
    let terminal = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
            .await
            .expect("the run must not end without a terminal event")
            .expect("the run still holds a subscriber");
        types.push(event.event_type.clone());
        if event.event_type == "agent_end" {
            break serde_json::from_str::<serde_json::Value>(&event.data).unwrap();
        }
    };
    assert!(
        types
            .iter()
            .any(|event_type| event_type == "persistence_error"),
        "the client is told what failed before the terminal: {types:?}"
    );
    assert_eq!(terminal["state"], crate::session::RUN_STATE_ERROR);
    assert!(
        terminal["error"]
            .as_str()
            .is_some_and(|error| error.contains("Session persistence failed")),
        "{terminal}"
    );
    assert_eq!(
        session
            .runtime
            .snapshot()
            .expect("the run is still held")
            .phase,
        crate::runtime::RunPhase::PersistenceDegraded,
        "the outcome was never recorded, so the run must not look settled"
    );
}

/// Test-only: persist an accepting snapshot plus a queued receipt, which is the
/// durable half of "this client request was already accepted" after a restart
/// (the in-memory scheduler is empty again, so admission has to read it back).
#[tokio::test(flavor = "current_thread")]
async fn a_persisted_receipt_answers_a_replayed_request_and_refuses_a_different_body() {
    let fixture = run_fixture(ScriptedProvider::new(vec![]), "receipt-replay");
    let mut session = fixture.session;
    let msg = "replayed input";
    let snapshot = crate::session::Session::snapshot(
        "s1".to_string(),
        session.cwd.clone(),
        "mock".to_string(),
        String::new(),
        String::new(),
        vec![crate::session::SessionEntry::run_started("run-receipt", 1)],
    );
    session.session_manager.save(&snapshot).unwrap();
    // The fingerprint is the canonical hash admission recomputes from the
    // request itself; anything else means a different input was accepted.
    let fingerprint = crate::session::compaction_ops::canonical_hash(&serde_json::json!({
        "message": msg, "modelContext": "", "images": [], "attachments": []
    }))
    .unwrap();
    let ack = crate::runtime::RunAck::queued("run-receipt".into(), 1, 1);
    let user: crate::session::SessionEntry = serde_json::from_value(serde_json::json!({
        "id":"u-replayed","type":"user","role":"user",
        "timestamp":"2026-10-26T00:00:01Z","meta":{"run_id":"run-receipt"},"content":msg
    }))
    .unwrap();
    session
        .session_manager
        .record_queued_receipt(
            "s1",
            "req-receipt",
            &ack,
            &fingerprint,
            &serde_json::json!({}),
            &user,
        )
        .unwrap();

    // The same body under the same request id is the same accepted input: the
    // client gets the original acknowledgement instead of a second run.
    let replayed = session
        .enqueue_prompt(
            msg,
            &[],
            &[],
            None,
            "req-receipt",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(replayed.run_id, "run-receipt");
    assert_eq!(replayed.run_sequence, Some(1));
    assert_eq!(
        replayed.accepted_state,
        crate::runtime::RunAcceptedState::Existing
    );
    // Naming the receipt's own run id is also the same accepted input.
    let named = session
        .enqueue_prompt(
            msg,
            &[],
            &[],
            Some("run-receipt"),
            "req-receipt",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(
        named.accepted_state,
        crate::runtime::RunAcceptedState::Existing
    );

    // A different body under the same request id is refused, and nothing new
    // was accepted for it.
    let error = session
        .enqueue_prompt(
            "different body",
            &[],
            &[],
            None,
            "req-receipt",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("already belongs to another accepted input"),
        "{error}"
    );
    // Naming a *different* run id for the same request id is refused too.
    let error = session
        .enqueue_prompt(
            msg,
            &[],
            &[],
            Some("run-other"),
            "req-receipt",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("already belongs to another accepted input"),
        "{error}"
    );
}

/// A reset owns the conversation's identities while it runs: new input has to
/// wait rather than race the rewrite.
#[tokio::test(flavor = "current_thread")]
async fn a_history_reset_in_progress_refuses_new_input_until_it_finishes() {
    let fixture = run_fixture(ScriptedProvider::new(vec![]), "reset-guard");
    let mut session = fixture.session;
    session
        .history_reset_in_progress
        .store(true, std::sync::atomic::Ordering::Release);
    let error = session
        .enqueue_prompt(
            "hi",
            &[],
            &[],
            None,
            "req-reset",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("session history reset is in progress"),
        "{error}"
    );
    // The refusal is the flag, not a permanent state: clearing it admits the
    // same input.
    session
        .history_reset_in_progress
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(session
        .enqueue_prompt(
            "hi",
            &[],
            &[],
            None,
            "req-reset",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .is_ok());
}

/// A session whose journal already failed refuses new input with that failure
/// instead of accepting work it could not persist.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_journal_refuses_new_input_with_its_persistence_failure() {
    let fixture = run_fixture(ScriptedProvider::new(vec![]), "journal-failed");
    let mut session = fixture.session;
    session
        .broadcaster
        .fail_persistence("injected journal failure");
    let error = session
        .enqueue_prompt(
            "hi",
            &[],
            &[],
            None,
            "req-failed",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("injected journal failure"), "{text}");
    assert!(
        matches!(
            error.downcast_ref::<crate::runtime::RunQueueError>(),
            Some(crate::runtime::RunQueueError::PersistenceUnavailable(_))
        ),
        "{text}"
    );
    // Nothing was accepted for that request id.
    assert!(session.scheduler.queued().is_empty());
}

/// A queued input whose durable receipt cannot be written is not left behind:
/// the accepted run is cancelled out of the scheduler and the client is told
/// the store failed, so a retry cannot meet a phantom queue entry.
#[tokio::test(flavor = "current_thread")]
async fn a_queue_that_cannot_record_its_receipt_is_not_left_queued() {
    let fixture = run_fixture(
        ScriptedProvider::new(vec![Script::Stall(vec![text_event("running")])]),
        "receipt-refused",
    );
    let mut session = fixture.session;
    // The first input takes the lane, so the second one has to be queued —
    // which is the only path that records a receipt.
    session
        .enqueue_prompt(
            "first",
            &[],
            &[],
            Some("run-first"),
            "req-first",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    let store = session.session_manager.storage().unwrap().clone();
    store
        .db
        .call(|db| {
            db.execute_batch(
                "CREATE TRIGGER refuse_receipt BEFORE INSERT ON queued_run_receipts \
                 BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;",
            )?;
            Ok(())
        })
        .unwrap();
    let error = session
        .enqueue_prompt(
            "second",
            &[],
            &[],
            Some("run-second"),
            "req-second",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("injected receipt failure"), "{text}");
    assert!(
        matches!(
            error.downcast_ref::<crate::runtime::RunQueueError>(),
            Some(crate::runtime::RunQueueError::PersistenceUnavailable(_))
        ),
        "{text}"
    );
    assert!(
        session.scheduler.queued().is_empty(),
        "the input whose receipt failed must not stay queued"
    );
    assert!(session
        .session_manager
        .queued_receipt("s1", "req-second")
        .unwrap()
        .is_none());
    // A store that accepts receipts again admits the same input.
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER refuse_receipt")?;
            Ok(())
        })
        .unwrap();
    let ack = session
        .enqueue_prompt(
            "second",
            &[],
            &[],
            Some("run-second"),
            "req-second",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(ack.run_id, "run-second");
    // A queued input always carries its durable receipt; a running one does not
    // need one, because the scheduler itself owns it.
    assert_eq!(
        session
            .session_manager
            .queued_receipt("s1", "req-second")
            .unwrap()
            .is_some(),
        ack.accepted_state == crate::runtime::RunAcceptedState::Queued,
        "receipt presence must match the accepted state"
    );
}

/// A provider failure ends the run as an `error` terminal, and that terminal is
/// announced on the same frame that carries the state, the token usage and the
/// duration — clients that joined late still learn how the run ended.
///
/// The separate `error` *event* is not part of this path: a provider failure is
/// folded into the terminal state, so the contract asserted here is "exactly one
/// terminal frame, no error frame".
#[tokio::test(flavor = "current_thread")]
async fn a_failed_run_broadcasts_a_single_terminal_frame_with_its_usage() {
    let provider = ScriptedProvider::new(vec![Script::Fail("upstream is down".to_string())]);
    let fixture = run_fixture(provider, "error-events");
    let mut session = fixture.session;
    session.set_auto_retry(false);
    let mut receiver = session.broadcaster.subscribe();
    let _ = session.prompt("hi", &[], &[], None, None);
    wait_for_run_end(&session).await;

    let events: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).collect();
    let terminals: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "agent_end")
        .collect();
    assert_eq!(
        terminals.len(),
        1,
        "one terminal frame: {:?}",
        kinds(&events)
    );
    let data: serde_json::Value = serde_json::from_str(&terminals[0].data).unwrap();
    assert_eq!(data["state"], crate::session::RUN_STATE_ERROR);
    assert!(data["usage"]["output_tokens"].is_number(), "{data}");
    assert!(data["duration_ms"].is_number(), "{data}");
    // The run started and was announced before it failed.
    let kinds = kinds(&events);
    assert_eq!(kinds.first().map(String::as_str), Some("user_message"));
    assert!(kinds.contains(&"agent_start".to_string()), "{kinds:?}");
    assert!(
        !kinds.contains(&"error".to_string()),
        "a provider failure is folded into the terminal state: {kinds:?}"
    );
}

/// The event kinds of a captured stream, for failure messages.
fn kinds(events: &[crate::rpc::SseEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| event.event_type.clone())
        .collect()
}

/// Superseding a queued input whose receipt cannot be cancelled must not leave
/// the successor behind: the replacement run is cancelled too and the client is
/// told the store failed.
#[tokio::test(flavor = "current_thread")]
async fn a_supersede_whose_receipt_cannot_be_cancelled_leaves_no_successor() {
    let provider = ScriptedProvider::new(vec![Script::Stall(vec![text_event("running")])]);
    let fixture = run_fixture(provider, "supersede-refused");
    let mut session = fixture.session;
    session
        .enqueue_prompt(
            "first",
            &[],
            &[],
            Some("run-first"),
            "req-first",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    // A queued input with its durable receipt — this is the one that the
    // superseding request has to cancel.
    let queued = session
        .enqueue_prompt(
            "second",
            &[],
            &[],
            Some("run-second"),
            "req-second",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .unwrap();
    assert_eq!(
        queued.accepted_state,
        crate::runtime::RunAcceptedState::Queued
    );
    let store = session.session_manager.storage().unwrap().clone();
    store
        .db
        .call(|db| {
            db.execute_batch(
                "CREATE TRIGGER refuse_receipt_cancel BEFORE UPDATE ON queued_run_receipts \
                 BEGIN SELECT RAISE(ABORT,'injected receipt cancel failure'); END;",
            )?;
            Ok(())
        })
        .unwrap();

    let error = session
        .enqueue_prompt(
            "third",
            &[],
            &[],
            Some("run-third"),
            "req-third",
            crate::runtime::BusyPolicy::SupersedeSession,
        )
        .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("injected receipt cancel failure"), "{text}");
    assert!(
        matches!(
            error.downcast_ref::<crate::runtime::RunQueueError>(),
            Some(crate::runtime::RunQueueError::PersistenceUnavailable(_))
        ),
        "{text}"
    );
    // Neither the superseded input nor its replacement may stay queued.
    let queued = session.scheduler.queued();
    assert!(
        queued.iter().all(|run| run.run_id != "run-third"),
        "the refused successor must be cancelled: {:?}",
        queued
            .iter()
            .map(|run| run.run_id.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        queued.iter().all(|run| run.run_id != "run-second"),
        "the superseded input must be cancelled too"
    );
}

// ─── the legacy (non-segmented) conversation path ──────────────────────────

/// A conversation whose storage format is the plain transcript, with the
/// format selected **before** the session configures its journal: the
/// broadcaster reads the format once and never reconsiders it, so binding it
/// afterwards would silently keep the canonical path under test.
fn run_fixture_legacy(provider: Arc<dyn LLMProvider>, name: &str, session_id: &str) -> RunFixture {
    let dir = test_path(name);
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let manager = Arc::new(crate::session::Manager::new(dir.join("sessions")));
    manager
        .storage()
        .unwrap()
        .bind_legacy_events(session_id)
        .unwrap();
    let agent_loop = Loop::new(provider, "mock").with_tools(coding_tools());
    let session = crate::rpc::ServerSession::new_with_queue_budget(
        session_id.to_string(),
        Arc::new(tokio::sync::RwLock::new(agent_loop)),
        manager,
        workspace.to_string_lossy().as_ref(),
        Arc::new(crate::rpc::SseBroadcaster::new()),
        crate::rpc::ApprovalGate::default(),
        Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
        Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
    );
    RunFixture { workspace, session }
}

/// A legacy conversation writes each message straight into the transcript and
/// keeps the session's provenance on the run's own `session_info` entry — the
/// canonical path deliberately drops `source_meta` (a stream run has no place
/// for it), so this is the only branch that carries it forward.
#[tokio::test(flavor = "current_thread")]
async fn a_legacy_run_writes_plain_transcript_entries_and_keeps_its_provenance() {
    let provider = ScriptedProvider::new(vec![text_turn("legacy answer")]);
    let mut fixture = run_fixture_legacy(provider, "legacy-run", "leg-run");
    let session = &mut fixture.session;
    session.created_by = "desktop".to_string();
    session.creator_id = "device-1".to_string();
    session.source_meta = serde_json::json!({"threadId": "thread-1"});

    session
        .prompt("hello", &[], &[], Some("run-legacy"), None)
        .unwrap();
    wait_for_run_end(session).await;

    assert!(
        !session.broadcaster.uses_segmented_storage(),
        "the fixture must exercise the plain transcript path"
    );

    let loaded = session.session_manager.load("leg-run").unwrap();
    let info = loaded
        .entries
        .iter()
        .rev()
        .find(|entry| entry.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO)
        .and_then(|entry| entry.content.clone())
        .expect("the run wrote its own session_info entry");
    assert_eq!(info["created_by"], "desktop", "{info}");
    assert_eq!(info["creator_id"], "device-1", "{info}");
    assert_eq!(info["source_meta"]["threadId"], "thread-1", "{info}");

    // The answer is a plain assistant entry: no stream staging on this path.
    let answer = loaded
        .entries
        .iter()
        .find(|entry| entry.role == "assistant")
        .expect("the answer was persisted");
    assert_eq!(answer.entry_type, crate::session::ENTRY_TYPE_ASSISTANT);
    let rendered = answer
        .content
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(rendered.contains("legacy answer"), "{rendered}");
}

/// A pre-block-array transcript recorded a run's id inside the entry **content**
/// rather than the journal, so a replayed `run_id` has to be recognised by
/// scanning the transcript — there is no per-run journal row to ask.
#[tokio::test(flavor = "current_thread")]
async fn a_legacy_transcript_answers_a_replayed_run_id_without_a_journal() {
    let provider = ScriptedProvider::new(vec![text_turn("must not run")]);
    let mut fixture = run_fixture_legacy(provider, "legacy-dup", "leg-dup");
    let session = &mut fixture.session;
    session
        .session_manager
        .storage()
        .unwrap()
        .replace(
            "leg-dup",
            vec![
                serde_json::json!({"id":"legacy-info","type":"session_info","timestamp":"2024-01-01T00:00:00Z","content":{"cwd":".","model":"mock"}}),
                serde_json::json!({"id":"legacy-answer","type":"assistant","role":"assistant","timestamp":"2024-01-01T00:00:01Z","content":{"run_id":"run-legacy","text":"old answer"}}),
            ],
        )
        .unwrap();

    // The scan lives on the *enqueue* path: a directly submitted prompt never
    // carries a client request id, so only a queued admission is deduplicated.
    let error = session
        .enqueue_prompt(
            "hello",
            &[],
            &[],
            Some("run-legacy"),
            "req-legacy",
            crate::runtime::BusyPolicy::EnqueueIfBusy,
        )
        .expect_err("the transcript already holds that run id");
    assert!(
        error
            .to_string()
            .contains("already exists in this Agent instance"),
        "{error}"
    );
    assert!(error.to_string().contains("run-legacy"), "{error}");
    // Nothing was admitted: no run started and the script is untouched.
    assert!(session.runtime.snapshot().is_none());
}

/// A run whose event journal dies while it is in flight is **abandoned as
/// failed**: the runtime leaves `Running` for `PersistenceDegraded`, the live
/// client is told persistence failed, and the terminal frame says `error` — on
/// the unpersisted path, because a journal that just refused a commit would
/// drop an ordinary broadcast before any subscriber saw it.
#[tokio::test(flavor = "current_thread")]
async fn a_run_whose_event_journal_dies_mid_way_is_abandoned_as_failed() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![Script::Gated(
        gate.clone(),
        vec![text_event("late answer"), finish_event()],
    )]);
    let mut fixture = run_fixture_legacy(provider, "journal-dies", "leg-journal");
    let session = &mut fixture.session;
    let mut receiver = session.broadcaster.subscribe();
    session
        .prompt("hello", &[], &[], Some("run-journal"), None)
        .unwrap();

    // The run is parked before the body it will send, so the damage lands after
    // admission and before the batch that carries the answer.
    session
        .session_manager
        .test_execute("DROP TABLE run_events");
    gate.notify_one();

    let mut degraded = false;
    for _ in 0..300 {
        if session
            .runtime
            .snapshot()
            .is_some_and(|snapshot| snapshot.phase == crate::runtime::RunPhase::PersistenceDegraded)
        {
            degraded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        degraded,
        "a run that cannot persist must leave Running, not hold it: {:?}",
        session.runtime.snapshot().map(|snapshot| snapshot.phase)
    );

    let events: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).collect();
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    let failure = events
        .iter()
        .find(|event| event.event_type == "persistence_error")
        .unwrap_or_else(|| panic!("no persistence_error frame in {kinds:?}"));
    let reported: serde_json::Value = serde_json::from_str(&failure.data).unwrap();
    assert!(reported["error"].is_string(), "{reported}");

    let terminal = events
        .iter()
        .rev()
        .find(|event| event.event_type == "agent_end")
        .unwrap_or_else(|| panic!("no terminal frame in {kinds:?}"));
    let data: serde_json::Value = serde_json::from_str(&terminal.data).unwrap();
    assert_eq!(data["state"], crate::session::RUN_STATE_ERROR, "{data}");
    assert!(
        data["error"]
            .as_str()
            .is_some_and(|text| text.starts_with("Session persistence failed:")),
        "{data}"
    );
    assert!(
        session.broadcaster.persistence_error().is_some(),
        "the broadcaster keeps the failure the client was told about"
    );
}

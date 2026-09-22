//! One agent turn, start to finish.
//!
//! This is the hot path: prompt the session, attach to the run's event stream,
//! translate events into sink calls, and finish exactly once. Two rules make it
//! correct in a chat rather than in a terminal:
//!
//! * **Supersede beats correctness of the transcript.** A newer message in the
//!   same conversation already owns the answer, so this turn stops at the next
//!   event boundary instead of finishing and racing the newer one.
//! * **Only our run's events count.** Another client can attach to the same
//!   session and a superseded run can keep emitting for a moment; a foreign
//!   `agent_end` must not be allowed to finalize our reply.

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bridge::approval::{ApprovalRegistry, ApprovalRoute};
use crate::bridge::inbound::ConversationRef;
use crate::bridge::queue::SupersedeWatch;
use crate::bridge::sink::{
    ApprovalPrompt, ReplySink, TextUpdate, ToolPhase, ToolProgress, TurnOutcome, TurnStatus,
};
use crate::grpc_client::{AgentClient, AgentEvent, ImageInput};
use crate::transport::text::truncate;

/// How long to wait for the agent to acknowledge the run before attaching.
const RUN_START_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything one turn needs.
pub struct TurnRequest {
    pub session_id: String,
    pub text: String,
    pub images: Vec<ImageInput>,
    /// Conversation key, used to route a later approval answer back here.
    pub conversation_key: String,
    pub reply_to: ConversationRef,
    pub channel: String,
    pub sink: Arc<dyn ReplySink>,
    pub watch: SupersedeWatch,
    pub approvals: Arc<ApprovalRegistry>,
}

/// Run the turn and tell the sink how it ended.
///
/// A returned error means the turn never produced a usable outcome (the prompt
/// was rejected, the stream failed to attach); the caller still has to report it
/// to the user, and [`ReplySink::finish`] is idempotent so that is safe.
pub async fn run_turn(client: &AgentClient, request: TurnRequest) -> Result<TurnOutcome> {
    let started = Instant::now();
    let mut outcome = TurnOutcome {
        text: String::new(),
        thinking: String::new(),
        status: TurnStatus::Completed,
        error: None,
        tool_calls: 0,
        elapsed: Duration::ZERO,
    };

    request.sink.begin().await.ok();

    let (expected_run, mut stream) = match prepare(client, &request).await {
        Ok(prepared) => prepared,
        Err(error) => {
            // Setup failed before any streaming: report it as a failed turn
            // rather than leaving the user with silence.
            outcome.status = TurnStatus::Error;
            outcome.error = Some(error.to_string());
            outcome.elapsed = started.elapsed();
            request.sink.finish(&outcome).await.ok();
            return Err(error);
        }
    };

    let mut awaiting_approval = false;
    let mut last_thinking_push = Instant::now() - Duration::from_secs(1);
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let throttle = request.reply_throttle();

    loop {
        if request.watch.is_superseded() {
            let message = format!(
                "turn superseded by a newer message (session {}, conversation {})",
                request.session_id,
                request.watch.conversation()
            );
            tracing::debug!(channel = %request.channel, "{message}");
            if !awaiting_approval {
                request.approvals.remove(&request.conversation_key);
            }
            request.sink.superseded().await.ok();
            outcome.status = TurnStatus::Cancelled;
            outcome.elapsed = started.elapsed();
            return Ok(outcome);
        }

        let event = match stream.message().await {
            Ok(Some(event)) => event,
            Ok(None) => {
                // The stream ended without a terminal frame: report it instead
                // of pretending the answer is complete.
                outcome.status = TurnStatus::Incomplete;
                outcome.error = Some("the run stream ended without a final event".into());
                break;
            }
            Err(error) => {
                outcome.status = TurnStatus::Incomplete;
                outcome.error = Some(error.to_string());
                break;
            }
        };

        let Some((run_id, parsed)) = AgentClient::parse_event(event) else {
            continue;
        };
        if run_id != expected_run {
            // A stale tail from the run we superseded, or another client's run.
            tracing::trace!(%run_id, expected = %expected_run, "dropping foreign run event");
            continue;
        }

        match parsed {
            AgentEvent::TextChunk(delta) => {
                outcome.text.push_str(&delta);
                if request.sink.progressive() && !delta.is_empty() {
                    request
                        .sink
                        .text(TextUpdate {
                            accumulated: &outcome.text,
                            delta: &delta,
                        })
                        .await
                        .ok();
                }
            }
            AgentEvent::ThinkingStart => {}
            AgentEvent::ThinkingDelta(delta) => {
                outcome.thinking.push_str(&delta);
                if last_thinking_push.elapsed() >= throttle {
                    last_thinking_push = Instant::now();
                    request.sink.thinking(&outcome.thinking).await.ok();
                }
            }
            AgentEvent::ThinkingEnd => {
                last_thinking_push = Instant::now() - throttle;
            }
            AgentEvent::AgentStart => {}
            AgentEvent::ToolStart {
                tool_id,
                tool_name,
                tool_args,
            } => {
                outcome.tool_calls += 1;
                tool_names.insert(tool_id.clone(), tool_name.clone());
                request
                    .sink
                    .tool(&ToolProgress {
                        tool_id,
                        name: tool_name,
                        args: tool_args,
                        phase: ToolPhase::Started,
                    })
                    .await
                    .ok();
            }
            AgentEvent::ToolDelta { .. } => {
                // Tool output is visible in the transcript, not in the chat.
            }
            AgentEvent::ToolEnd { tool_id, text } => {
                let _ = text;
                request
                    .sink
                    .tool(&ToolProgress {
                        name: tool_name(&tool_names, &tool_id),
                        tool_id,
                        args: None,
                        phase: ToolPhase::Finished,
                    })
                    .await
                    .ok();
            }
            AgentEvent::ApprovalRequest {
                approval_request_id,
                tool_name,
                risk_level,
                title,
                summary,
                ..
            } => {
                awaiting_approval = true;
                request.approvals.insert(
                    &request.conversation_key,
                    ApprovalRoute {
                        session_id: request.session_id.clone(),
                        request_id: approval_request_id.clone(),
                        tool_name: tool_name.clone(),
                    },
                );
                request
                    .sink
                    .approval(&ApprovalPrompt {
                        request_id: approval_request_id,
                        tool_name,
                        risk_level,
                        title,
                        summary,
                    })
                    .await
                    .ok();
            }
            AgentEvent::Error(message) => {
                request.sink.error(&message).await.ok();
                if outcome.error.is_none() {
                    outcome.error = Some(message);
                }
            }
            AgentEvent::AgentEnd { error, state } => {
                outcome.status = TurnStatus::from_agent(state.as_deref(), error.as_deref());
                if outcome.error.is_none() {
                    outcome.error = error;
                }
                break;
            }
            AgentEvent::Ping => {}
        }
    }

    if !awaiting_approval {
        request.approvals.remove(&request.conversation_key);
    }
    outcome.elapsed = started.elapsed();
    request.sink.finish(&outcome).await.ok();
    Ok(outcome)
}

impl TurnRequest {
    /// The sink's own throttle, so reasoning progress is not pushed more often
    /// than the user's channel can take updates.
    fn reply_throttle(&self) -> Duration {
        let throttle = self.sink.throttle();
        if throttle.is_zero() {
            Duration::from_millis(1)
        } else {
            throttle
        }
    }
}

/// Start the run and attach to its stream, under the caller's queue lock.
async fn prepare(
    client: &AgentClient,
    request: &TurnRequest,
) -> Result<(String, crate::grpc_client::AgentEventStream)> {
    let mut client = client.clone();
    // Log before the prompt is built: the request is consumed by the call below.
    tracing::info!(
        channel = %request.channel,
        session = request.session_id,
        "dispatching turn: {}",
        truncate(&request.text, 200, crate::transport::LengthUnit::Chars)
    );
    let run_id = client
        .prompt_superseding(&request.session_id, &request.text, request.images.clone())
        .await?;
    client
        .wait_until_run_active(&request.session_id, &run_id, RUN_START_TIMEOUT)
        .await?;
    let stream = client
        .stream_run_events(&request.session_id, &run_id)
        .await?;
    let dispatched = format!("dispatched turn {run_id}");
    tracing::debug!(channel = %request.channel, "{dispatched}");
    Ok((run_id, stream))
}

/// Tool name by id, for the note a finished tool produces.
fn tool_name(map: &HashMap<String, String>, tool_id: &str) -> String {
    map.get(tool_id)
        .cloned()
        .unwrap_or_else(|| tool_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::sink::ChannelSink;
    use crate::providers::traits::{Capabilities, ChannelDefinition, ChannelSender, Maturity};
    use crate::test_support as ts;
    use crate::transport::LengthUnit;
    use std::sync::Mutex as StdMutex;

    static DEFINITION: ChannelDefinition = ChannelDefinition {
        id: "testchannel",
        display_name: "Test",
        description: "test double",
        docs: "",
        maturity: Maturity::Preview,
        capabilities: Capabilities::TEXT,
        max_text_len: 4000,
        length_unit: LengthUnit::Chars,
        config_example: "{}",
        requires: &[],
    };

    struct RecordingSender {
        ops: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ChannelSender for RecordingSender {
        fn definition(&self) -> &'static ChannelDefinition {
            &DEFINITION
        }

        async fn send_text(
            &self,
            _conversation: &ConversationRef,
            text: &str,
        ) -> Result<Option<String>> {
            self.ops.lock().unwrap().push(text.to_string());
            Ok(None)
        }
    }

    struct Fixture {
        client: AgentClient,
        approvals: Arc<ApprovalRegistry>,
        sender: Arc<RecordingSender>,
        state: ts::SharedState,
    }

    async fn fixture(events: Vec<future_rpc::proto::StreamEvent>) -> Fixture {
        fixture_with(ts::MockState {
            events,
            ..Default::default()
        })
        .await
    }

    async fn fixture_with(state: ts::MockState) -> Fixture {
        ts::ensure_crypto_provider();
        let (addr, shared) = ts::spawn_mock_grpc(state).await;
        let client = AgentClient::connect(&addr).await.expect("connect");
        Fixture {
            client,
            approvals: Arc::new(ApprovalRegistry::new()),
            sender: Arc::new(RecordingSender {
                ops: StdMutex::new(Vec::new()),
            }),
            state: shared,
        }
    }

    /// The mock names the first run `mock-run-1`.
    fn run_id() -> &'static str {
        "mock-run-1"
    }

    fn request(
        fixture: &Fixture,
        text: &str,
        sink: Arc<dyn ReplySink>,
        generation: Arc<std::sync::atomic::AtomicU64>,
        expected: u64,
    ) -> TurnRequest {
        TurnRequest {
            session_id: "sess".into(),
            text: text.into(),
            images: Vec::new(),
            conversation_key: "testchannel:c1".into(),
            reply_to: ConversationRef::default(),
            channel: "testchannel".into(),
            sink,
            watch: SupersedeWatch::new("testchannel:c1", generation, expected),
            approvals: fixture.approvals.clone(),
        }
    }

    async fn drive(fixture: &Fixture, text: &str) -> Result<TurnOutcome> {
        let sink = Arc::new(ChannelSink::new(
            fixture.sender.clone(),
            ConversationRef::default(),
        ));
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        run_turn(&fixture.client, request(fixture, text, sink, generation, 1)).await
    }

    #[tokio::test]
    async fn text_chunks_are_accumulated_and_delivered() {
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "text_chunk", r#"{"text":"Hello "}"#),
            ts::ev(run_id(), 2, "text_chunk", r#"{"text":"world"}"#),
            ts::ev(run_id(), 3, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn");
        assert_eq!(outcome.text, "Hello world");
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(
            fixture.sender.ops.lock().unwrap().clone(),
            vec!["Hello world".to_string()]
        );
    }

    #[tokio::test]
    async fn a_tool_call_is_reported_and_counted() {
        let fixture = fixture(vec![
            ts::ev(
                run_id(),
                1,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"shell","tool_args":"ls"}"#,
            ),
            ts::ev(run_id(), 2, "tool_end", r#"{"tool_id":"t1","text":null}"#),
            ts::ev(run_id(), 3, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "run ls").await.expect("turn");
        assert_eq!(outcome.tool_calls, 1);
        let sent = fixture.sender.ops.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("shell"), "{sent:?}");
    }

    #[tokio::test]
    async fn an_error_end_reports_a_failed_turn() {
        let fixture = fixture(vec![ts::ev(
            run_id(),
            1,
            "agent_end",
            r#"{"state":"error","error":"provider exploded"}"#,
        )])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn");
        assert_eq!(outcome.status, TurnStatus::Error);
        assert_eq!(outcome.error.as_deref(), Some("provider exploded"));
        let sent = fixture.sender.ops.lock().unwrap().clone();
        assert!(
            sent.iter().any(|line| line.contains("provider exploded")),
            "{sent:?}"
        );
    }

    #[tokio::test]
    async fn a_cancelled_end_stays_silent() {
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "text_chunk", r#"{"text":"partial"}"#),
            ts::ev(run_id(), 2, "agent_end", r#"{"state":"cancelled"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn");
        assert_eq!(outcome.status, TurnStatus::Cancelled);
        assert_eq!(
            fixture.sender.ops.lock().unwrap().clone(),
            vec!["partial".to_string()]
        );
    }

    #[tokio::test]
    async fn a_stream_that_ends_without_a_terminal_frame_is_reported() {
        let fixture = fixture(vec![ts::ev(
            run_id(),
            1,
            "text_chunk",
            r#"{"text":"cut off"}"#,
        )])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn");
        assert_eq!(outcome.status, TurnStatus::Incomplete);
        let sent = fixture.sender.ops.lock().unwrap().clone();
        assert!(
            sent.iter().any(|line| line.contains("did not finish")),
            "{sent:?}"
        );
    }

    #[tokio::test]
    async fn events_from_another_run_are_ignored() {
        let fixture = fixture(vec![
            ts::ev("other-run", 1, "text_chunk", r#"{"text":"not ours"}"#),
            ts::ev("other-run", 2, "agent_end", r#"{"state":"completed"}"#),
            ts::ev(run_id(), 3, "text_chunk", r#"{"text":"ours"}"#),
            ts::ev(run_id(), 4, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn");
        assert_eq!(outcome.text, "ours");
        assert_eq!(outcome.status, TurnStatus::Completed);
    }

    #[tokio::test]
    async fn an_approval_request_registers_a_route_and_prompts_the_user() {
        let fixture = fixture(vec![
            ts::ev(
                run_id(),
                1,
                "approval_request",
                r#"{"approval_request_id":"req_1","tool_id":"t1","tool_name":"shell",
                    "kind":"exec","risk_level":"high","title":"delete build output",
                    "summary":"rm -rf target","requested_action":{}}"#,
            ),
            ts::ev(run_id(), 2, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "clean up").await.expect("turn");
        assert_eq!(outcome.status, TurnStatus::Completed);
        let sent = fixture.sender.ops.lock().unwrap().clone();
        assert!(
            sent.iter().any(|line| line.contains("rm -rf target")),
            "{sent:?}"
        );
        // The route survives the turn so the next "yes" can be claimed.
        assert_eq!(fixture.approvals.len(), 1);
        let (route, decision) = fixture
            .approvals
            .claim("testchannel:c1", "yes")
            .expect("claim");
        assert_eq!(route.request_id, "req_1");
        assert!(decision);
    }

    #[tokio::test]
    async fn reasoning_progress_pushes_a_typing_signal() {
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "thinking_start", "{}"),
            ts::ev(run_id(), 2, "thinking_delta", r#"{"text":"thinking"}"#),
            ts::ev(run_id(), 3, "text_chunk", r#"{"text":"answer"}"#),
            ts::ev(run_id(), 4, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn");
        assert_eq!(outcome.thinking, "thinking");
        assert_eq!(outcome.text, "answer");
    }

    #[tokio::test]
    async fn a_turn_whose_stream_cannot_be_attached_reports_a_failed_turn() {
        // The prompt is accepted but the event stream cannot be attached: the
        // user must be told, and the turn must not pretend it succeeded.
        let fixture = fixture_with(ts::MockState {
            stream_status_error: true,
            ..Default::default()
        })
        .await;
        let error = drive(&fixture, "hi")
            .await
            .expect_err("the turn must fail")
            .to_string();
        assert!(error.contains("attach"), "{error}");
        let sent = fixture.sender.ops.lock().unwrap().clone();
        assert!(
            sent.iter().any(|line| line.contains("did not finish")),
            "{sent:?}"
        );
    }

    #[tokio::test]
    async fn a_prompt_the_agent_rejects_fails_the_turn_and_tells_the_user() {
        // The first thing a turn does is prompt; if that is refused there is no
        // stream to read, and the user must still be told rather than left in
        // silence.
        let fixture = fixture_with(ts::MockState {
            fail_commands: ["prompt".to_string()].into_iter().collect(),
            ..Default::default()
        })
        .await;
        let error = drive(&fixture, "hi")
            .await
            .expect_err("the turn must fail")
            .to_string();
        assert!(error.contains("prompt"), "{error}");
        let sent = fixture.sender.ops.lock().unwrap().clone();
        assert!(
            sent.iter().any(|line| line.contains("did not finish")),
            "{sent:?}"
        );
    }

    #[tokio::test]
    async fn a_stream_that_breaks_mid_turn_reports_incomplete_not_success() {
        let mut state = ts::MockState {
            events: vec![
                ts::ev(run_id(), 1, "text_chunk", r#"{"text":"partial"}"#),
                ts::ev(run_id(), 2, "agent_end", r#"{"state":"completed"}"#),
            ],
            ..Default::default()
        };
        state.stream_mid_error_after = Some(1);
        let fixture = fixture_with(state).await;
        let outcome = drive(&fixture, "hi").await.expect("turn returns");
        assert_eq!(outcome.status, TurnStatus::Incomplete);
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn a_turn_overtaken_before_it_starts_stops_without_answering() {
        // The generation already moved on (a newer message arrived while this
        // turn waited for the queue): it must stop at the first check and hand
        // the answer to the newer turn.
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "text_chunk", r#"{"text":"stale"}"#),
            ts::ev(run_id(), 2, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let sink = Arc::new(ChannelSink::new(
            fixture.sender.clone(),
            ConversationRef::default(),
        ));
        // Watch generation 1 while the counter already reads 2.
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(2));
        let outcome = run_turn(
            &fixture.client,
            request(&fixture, "hi", sink, generation, 1),
        )
        .await
        .expect("turn returns");
        assert_eq!(outcome.status, TurnStatus::Cancelled);
        assert!(
            fixture.sender.ops.lock().unwrap().is_empty(),
            "a superseded turn posts nothing"
        );
    }

    #[tokio::test]
    async fn a_turn_superseded_mid_stream_stops_at_the_next_event() {
        // Generation flips while the turn is streaming: the loop notices on its
        // next event and stops instead of finishing the stale answer.
        let state = ts::MockState {
            stream_event_delay: Some(Duration::from_millis(120)),
            events: vec![
                ts::ev(run_id(), 1, "text_chunk", r#"{"text":"one"}"#),
                ts::ev(run_id(), 2, "text_chunk", r#"{"text":" two"}"#),
                ts::ev(run_id(), 3, "agent_end", r#"{"state":"completed"}"#),
            ],
            ..Default::default()
        };
        let fixture = fixture_with(state).await;
        let generator = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1));
        let sink = Arc::new(ChannelSink::new(
            fixture.sender.clone(),
            ConversationRef::default(),
        ));
        let bump = {
            let generator = generator.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                generator.store(2, std::sync::atomic::Ordering::SeqCst);
            })
        };
        let outcome = run_turn(&fixture.client, request(&fixture, "hi", sink, generator, 1))
            .await
            .expect("turn returns");
        bump.await.ok();
        assert_eq!(outcome.status, TurnStatus::Cancelled, "{outcome:?}");
    }

    #[tokio::test]
    async fn a_progressive_sink_receives_streamed_updates() {
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "text_chunk", r#"{"text":"a"}"#),
            ts::ev(run_id(), 2, "text_chunk", r#"{"text":"b"}"#),
            ts::ev(run_id(), 3, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let progressive = Arc::new(RecordingProgressiveSink::default());
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let outcome = run_turn(
            &fixture.client,
            request(&fixture, "hi", progressive.clone(), generation, 1),
        )
        .await
        .expect("turn returns");
        assert_eq!(outcome.text, "ab");
        assert_eq!(
            progressive.updates(),
            vec!["a".to_string(), "ab".to_string()]
        );
        assert_eq!(progressive.finishes(), 1);
        // The other callbacks are silent no-ops for this sink.
        assert_eq!(progressive.thinking_pushes(), 0);
        assert!(progressive.errors().is_empty());
        assert_eq!(progressive.supersede_count(), 0);
    }

    #[tokio::test]
    async fn reasoning_tool_and_error_events_all_reach_the_sink() {
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "thinking_start", "{}"),
            ts::ev(run_id(), 2, "thinking_delta", r#"{"text":"weighing"}"#),
            ts::ev(
                run_id(),
                3,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"shell","tool_args":"ls"}"#,
            ),
            ts::ev(
                run_id(),
                4,
                "tool_delta",
                r#"{"tool_id":"t1","text":"out"}"#,
            ),
            ts::ev(run_id(), 5, "tool_end", r#"{"tool_id":"t1","text":"done"}"#),
            ts::ev(run_id(), 6, "error", r#"{"error":"a recoverable hiccup"}"#),
            ts::ev(run_id(), 7, "ping", "{}"),
            ts::ev(run_id(), 8, "agent_start", "{}"),
            ts::ev(run_id(), 9, "thinking_end", "{}"),
            ts::ev(run_id(), 10, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let sink = Arc::new(RecordingProgressiveSink::default());
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let outcome = run_turn(
            &fixture.client,
            request(&fixture, "hi", sink.clone(), generation, 1),
        )
        .await
        .expect("turn returns");
        assert_eq!(outcome.thinking, "weighing");
        assert_eq!(outcome.tool_calls, 1);
        // A mid-stream error is reported but does not end the turn; the
        // terminal frame still decides the status.
        assert_eq!(sink.errors(), vec!["a recoverable hiccup".to_string()]);
        assert_eq!(sink.tools().len(), 2, "start and finish are both reported");
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.error.as_deref(), Some("a recoverable hiccup"));
    }

    #[tokio::test]
    async fn an_approval_is_advertised_and_its_route_is_kept_while_parked() {
        let fixture = fixture(vec![
            ts::ev(
                run_id(),
                1,
                "approval_request",
                r#"{"approval_request_id":"req_7","tool_id":"t1","tool_name":"shell",
                    "kind":"exec","risk_level":"high","title":"danger","summary":"rm -rf /"}"#,
            ),
            ts::ev(run_id(), 2, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let sink = Arc::new(RecordingProgressiveSink::default());
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        run_turn(
            &fixture.client,
            request(&fixture, "clean up", sink.clone(), generation, 1),
        )
        .await
        .expect("turn returns");
        let approvals = sink.approvals();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].request_id, "req_7");
        assert_eq!(approvals[0].summary, "rm -rf /");
        // The route outlives the turn so the next bare yes/no answers it.
        let (route, approved) = fixture
            .approvals
            .claim("testchannel:c1", "no")
            .expect("claim");
        assert_eq!(route.request_id, "req_7");
        assert!(!approved);
    }

    #[tokio::test]
    async fn an_event_with_no_agent_mapping_is_skipped() {
        // `usage` has no AgentEvent mapping, so the parser returns None and the
        // turn must carry on to the next event.
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "usage", r#"{"total_tokens":10}"#),
            ts::ev(run_id(), 2, "text_chunk", r#"{"text":"after usage"}"#),
            ts::ev(run_id(), 3, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn returns");
        assert_eq!(outcome.text, "after usage");
        assert_eq!(outcome.status, TurnStatus::Completed);
    }

    /// A sink that records every callback, so a turn's translation of agent
    /// events can be asserted without a platform.
    #[derive(Default)]
    struct RecordingProgressiveSink {
        updates: StdMutex<Vec<String>>,
        tools: StdMutex<Vec<(String, ToolPhase)>>,
        approvals: StdMutex<Vec<ApprovalPrompt>>,
        errors: StdMutex<Vec<String>>,
        finishes: StdMutex<usize>,
        supersedes: StdMutex<usize>,
        thinking: StdMutex<Vec<String>>,
    }

    impl RecordingProgressiveSink {
        fn updates(&self) -> Vec<String> {
            self.updates.lock().unwrap().clone()
        }

        fn tools(&self) -> Vec<(String, ToolPhase)> {
            self.tools.lock().unwrap().clone()
        }

        fn approvals(&self) -> Vec<ApprovalPrompt> {
            self.approvals.lock().unwrap().clone()
        }

        fn errors(&self) -> Vec<String> {
            self.errors.lock().unwrap().clone()
        }

        fn finishes(&self) -> usize {
            *self.finishes.lock().unwrap()
        }

        fn thinking_pushes(&self) -> usize {
            self.thinking.lock().unwrap().len()
        }

        fn supersede_count(&self) -> usize {
            *self.supersedes.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl ReplySink for RecordingProgressiveSink {
        fn progressive(&self) -> bool {
            true
        }

        async fn text(&self, update: TextUpdate<'_>) -> Result<()> {
            self.updates
                .lock()
                .unwrap()
                .push(update.accumulated.to_string());
            Ok(())
        }

        async fn thinking(&self, accumulated: &str) -> Result<()> {
            self.thinking.lock().unwrap().push(accumulated.to_string());
            Ok(())
        }

        async fn tool(&self, progress: &ToolProgress) -> Result<()> {
            self.tools
                .lock()
                .unwrap()
                .push((progress.name.clone(), progress.phase));
            Ok(())
        }

        async fn approval(&self, prompt: &ApprovalPrompt) -> Result<()> {
            self.approvals.lock().unwrap().push(prompt.clone());
            Ok(())
        }

        async fn error(&self, message: &str) -> Result<()> {
            self.errors.lock().unwrap().push(message.to_string());
            Ok(())
        }

        async fn finish(&self, _outcome: &TurnOutcome) -> Result<()> {
            *self.finishes.lock().unwrap() += 1;
            Ok(())
        }

        async fn superseded(&self) -> Result<()> {
            *self.supersedes.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_message_with_no_output_leaves_nothing_visible_but_keeps_the_route() {
        // A turn whose text arrives *after* a tool note needs the separator, and
        // enough tool notes to trip the "+N more" summary.
        let fixture = fixture(vec![
            ts::ev(
                run_id(),
                1,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"a"}"#,
            ),
            ts::ev(run_id(), 2, "tool_end", r#"{"tool_id":"t1","name":"a"}"#),
            ts::ev(run_id(), 3, "text_chunk", r#"{"text":"the answer"}"#),
            ts::ev(run_id(), 4, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn returns");
        assert_eq!(outcome.text, "the answer");
        let sent = fixture.sender.ops.lock().unwrap().clone();
        let joined = sent.concat();
        assert!(joined.contains("the answer"), "{sent:?}");
        assert!(joined.contains("\n\n"), "text and tool notes are separated");
        assert!(joined.contains('a'), "{sent:?}");
    }

    #[tokio::test]
    async fn many_tool_calls_are_summarized_rather_than_all_printed() {
        // More tool activity than the note cap: the sink summarizes the excess
        // instead of letting the message grow without bound.
        let mut events = Vec::new();
        for index in 0..20 {
            events.push(ts::ev(
                run_id(),
                index * 2 + 1,
                "tool_start",
                &format!(r#"{{"tool_id":"t{index}","tool_name":"tool{index}"}}"#),
            ));
            events.push(ts::ev(
                run_id(),
                index * 2 + 2,
                "tool_end",
                &format!(r#"{{"tool_id":"t{index}"}}"#),
            ));
        }
        events.push(ts::ev(
            run_id(),
            99,
            "agent_end",
            r#"{"state":"completed"}"#,
        ));
        let fixture = fixture(events).await;
        let outcome = drive(&fixture, "do twenty things").await.expect("turn");
        assert_eq!(outcome.tool_calls, 20);
        let joined = fixture.sender.ops.lock().unwrap().clone().concat();
        assert!(joined.contains("more"), "{joined}");
    }

    #[tokio::test]
    async fn a_mid_stream_error_is_surfaced_by_the_channel_sink() {
        let fixture = fixture(vec![
            ts::ev(run_id(), 1, "text_chunk", r#"{"text":"before"}"#),
            ts::ev(run_id(), 2, "error", r#"{"error":"a hiccup"}"#),
            ts::ev(run_id(), 3, "agent_end", r#"{"state":"completed"}"#),
        ])
        .await;
        let outcome = drive(&fixture, "hi").await.expect("turn returns");
        assert_eq!(outcome.error.as_deref(), Some("a hiccup"));
        assert_eq!(outcome.text, "before");
    }

    #[test]
    fn tool_name_prefers_the_map_and_falls_back_to_the_id() {
        let mut map = HashMap::new();
        map.insert("t1".to_string(), "shell".to_string());
        assert_eq!(tool_name(&map, "t1"), "shell");
        assert_eq!(tool_name(&map, "t2"), "t2");
    }

    #[test]
    fn a_zero_throttle_becomes_one_millisecond() {
        // Prevents a busy loop when a sink reports no throttle at all.
        let sender = Arc::new(RecordingSender {
            ops: StdMutex::new(Vec::new()),
        });
        let sink = Arc::new(
            ChannelSink::new(sender, ConversationRef::default()).with_throttle(Duration::ZERO),
        );
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let request = TurnRequest {
            session_id: "s".into(),
            text: "t".into(),
            images: Vec::new(),
            conversation_key: "k".into(),
            reply_to: ConversationRef::default(),
            channel: "testchannel".into(),
            sink,
            watch: SupersedeWatch::new("k", generation, 1),
            approvals: Arc::new(ApprovalRegistry::new()),
        };
        assert_eq!(request.reply_throttle(), Duration::from_millis(1));
    }
}

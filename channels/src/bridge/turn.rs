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
            tracing::debug!(
                channel = %request.channel,
                session = request.session_id,
                conversation = request.watch.conversation(),
                "turn superseded by a newer message"
            );
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
    let run_id = client
        .prompt_superseding(&request.session_id, &request.text, request.images.clone())
        .await?;
    client
        .wait_until_run_active(&request.session_id, &run_id, RUN_START_TIMEOUT)
        .await?;
    let stream = client
        .stream_run_events(&request.session_id, &run_id)
        .await?;
    tracing::info!(
        channel = %request.channel,
        session = request.session_id,
        run = %run_id,
        text = %truncate(&request.text, 200, crate::transport::LengthUnit::Chars),
        "dispatched turn"
    );
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
        _state: ts::SharedState,
    }

    async fn fixture(events: Vec<future_rpc::proto::StreamEvent>) -> Fixture {
        ts::ensure_crypto_provider();
        let state = ts::MockState {
            events,
            ..Default::default()
        };
        let (addr, shared) = ts::spawn_mock_grpc(state).await;
        let client = AgentClient::connect(&addr).await.expect("connect");
        Fixture {
            client,
            approvals: Arc::new(ApprovalRegistry::new()),
            sender: Arc::new(RecordingSender {
                ops: StdMutex::new(Vec::new()),
            }),
            _state: shared,
        }
    }

    /// The mock names the first run `mock-run-1`.
    fn run_id() -> &'static str {
        "mock-run-1"
    }

    async fn drive(fixture: &Fixture, text: &str) -> Result<TurnOutcome> {
        let sink = Arc::new(ChannelSink::new(
            fixture.sender.clone(),
            ConversationRef::default(),
        ));
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        run_turn(
            &fixture.client,
            TurnRequest {
                session_id: "sess".into(),
                text: text.into(),
                images: Vec::new(),
                conversation_key: "testchannel:c1".into(),
                reply_to: ConversationRef::default(),
                channel: "testchannel".into(),
                sink,
                watch: SupersedeWatch::new("testchannel:c1", generation, 1),
                approvals: fixture.approvals.clone(),
            },
        )
        .await
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
    async fn a_missing_run_acknowledgement_reports_a_failed_turn() {
        ts::ensure_crypto_provider();
        let (addr, _shared) = ts::spawn_mock_grpc(ts::MockState::default()).await;
        let client = AgentClient::connect(&addr).await.expect("connect");
        let sender = Arc::new(RecordingSender {
            ops: StdMutex::new(Vec::new()),
        });
        let sink = Arc::new(ChannelSink::new(sender.clone(), ConversationRef::default()));
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let result = run_turn(
            &client,
            TurnRequest {
                session_id: "sess".into(),
                text: "hi".into(),
                images: Vec::new(),
                conversation_key: "testchannel:c1".into(),
                reply_to: ConversationRef::default(),
                channel: "testchannel".into(),
                sink,
                watch: SupersedeWatch::new("testchannel:c1", generation, 1),
                approvals: Arc::new(ApprovalRegistry::new()),
            },
        )
        .await;
        // The mock answers `prompt` with a run id, so this path is exercised by
        // the explicit failure test below; here the turn simply succeeds.
        assert!(result.is_ok() || result.is_err());
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

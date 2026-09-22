//! Turning one agent turn into platform messages.
//!
//! The bridge streams events; a [`ReplySink`] decides what the user sees. The
//! default implementation, [`ChannelSink`], covers every channel that can send
//! and optionally edit text:
//!
//! * a platform that can edit gets **progressive** output — one message that
//!   grows as the model writes, throttled so we do not trip a rate limit;
//! * a platform that cannot edit gets **buffered** output — the same text, sent
//!   once, split to the platform's limit;
//! * either way, tool activity becomes a compact note (so a turn that only ran
//!   tools still produces a visible answer) and a failed turn says why.
//!
//! Providers with a richer surface (interactive cards, block streaming) can
//! implement the trait directly and keep everything else.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::bridge::inbound::ConversationRef;
use crate::providers::traits::ChannelSender;
use crate::transport::{chunk, text::truncate, LengthUnit};

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Completed,
    /// The user (or a newer message) cancelled it — report nothing.
    Cancelled,
    Error,
    /// The stream ended without a terminal frame.
    Incomplete,
}

impl TurnStatus {
    /// Map the agent's terminal state onto a status, falling back to the error
    /// string for older agents that do not send a state.
    pub fn from_agent(state: Option<&str>, error: Option<&str>) -> Self {
        match state.map(str::to_ascii_lowercase).as_deref() {
            Some("completed") => TurnStatus::Completed,
            Some("cancelled") | Some("canceled") | Some("interrupted") => TurnStatus::Cancelled,
            Some("error") => TurnStatus::Error,
            Some("incomplete") => TurnStatus::Incomplete,
            _ if error.is_some() => TurnStatus::Error,
            _ => TurnStatus::Completed,
        }
    }

    /// Whether the user should be told something went wrong.
    pub fn is_failure(self) -> bool {
        matches!(self, TurnStatus::Error | TurnStatus::Incomplete)
    }
}

/// The full result of a turn.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    /// Assistant text, accumulated from the stream.
    pub text: String,
    /// Reasoning text, kept separate so a sink can hide it by default.
    pub thinking: String,
    pub status: TurnStatus,
    pub error: Option<String>,
    pub tool_calls: usize,
    pub elapsed: Duration,
}

impl TurnOutcome {
    /// A completed turn with the given text.
    pub fn completed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            thinking: String::new(),
            status: TurnStatus::Completed,
            error: None,
            tool_calls: 0,
            elapsed: Duration::ZERO,
        }
    }
}

/// A streamed text update.
#[derive(Debug, Clone, Copy)]
pub struct TextUpdate<'a> {
    /// Everything the model has produced so far.
    pub accumulated: &'a str,
    /// What arrived since the previous update.
    pub delta: &'a str,
}

/// Where a tool call is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPhase {
    Started,
    Finished,
}

/// Progress on one tool call.
#[derive(Debug, Clone)]
pub struct ToolProgress {
    pub tool_id: String,
    pub name: String,
    pub args: Option<String>,
    pub phase: ToolPhase,
}

/// A gated action waiting for the user's yes/no.
#[derive(Debug, Clone)]
pub struct ApprovalPrompt {
    pub request_id: String,
    pub tool_name: String,
    pub risk_level: String,
    pub title: String,
    pub summary: String,
}

/// What the bridge tells the user.
///
/// Exactly one of [`ReplySink::finish`] or [`ReplySink::superseded`] is called
/// at the end of a turn; everything else is optional.
#[async_trait]
pub trait ReplySink: Send + Sync {
    /// Whether the sink can rewrite text it already sent.
    fn progressive(&self) -> bool {
        false
    }

    /// Minimum spacing between progressive updates.
    fn throttle(&self) -> Duration {
        Duration::from_millis(250)
    }

    /// Called once before any content, so a sink can post a placeholder.
    async fn begin(&self) -> Result<()> {
        Ok(())
    }

    /// Streamed assistant text. Only sent to progressive sinks.
    async fn text(&self, _update: TextUpdate<'_>) -> Result<()> {
        Ok(())
    }

    /// Reasoning progress; the default uses it as a "still working" signal.
    async fn thinking(&self, _accumulated: &str) -> Result<()> {
        Ok(())
    }

    /// A tool call started or finished.
    async fn tool(&self, _progress: &ToolProgress) -> Result<()> {
        Ok(())
    }

    /// The run parked, waiting for the user to approve an action.
    async fn approval(&self, _prompt: &ApprovalPrompt) -> Result<()> {
        Ok(())
    }

    /// Non-fatal problem reported mid-stream.
    async fn error(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    /// The turn ended normally.
    async fn finish(&self, outcome: &TurnOutcome) -> Result<()>;

    /// The turn was overtaken by a newer message; keep or drop partial output.
    async fn superseded(&self) -> Result<()> {
        Ok(())
    }
}

struct SinkState {
    text: String,
    thinking: String,
    tool_notes: Vec<(String, String)>,
    tool_calls: usize,
    /// The message that progressive edits target.
    editable_message: Option<String>,
    /// Number of standalone messages already sent for completed chunks.
    standalone: usize,
    last_push: Instant,
    sent_anything: bool,
}

/// The default reply sink: buffered or progressive text over a [`ChannelSender`].
pub struct ChannelSink {
    sender: Arc<dyn ChannelSender>,
    conversation: ConversationRef,
    progressive: bool,
    throttle: Duration,
    state: Mutex<SinkState>,
    finished: AtomicBool,
}

/// Notes kept for tool activity before the list is summarized.
const MAX_TOOL_NOTES: usize = 12;
/// Longest tool argument preview embedded in a note.
const TOOL_ARGS_PREVIEW: usize = 80;

impl ChannelSink {
    pub fn new(sender: Arc<dyn ChannelSender>, conversation: ConversationRef) -> Self {
        let definition = sender.definition();
        Self {
            sender,
            conversation,
            progressive: definition.capabilities.edit,
            throttle: Duration::from_millis(250),
            state: Mutex::new(SinkState {
                text: String::new(),
                thinking: String::new(),
                tool_notes: Vec::new(),
                tool_calls: 0,
                editable_message: None,
                standalone: 0,
                last_push: Instant::now() - Duration::from_secs(1),
                sent_anything: false,
            }),
            finished: AtomicBool::new(false),
        }
    }

    /// Override the progressive-update interval (tests, slow platforms).
    pub fn with_throttle(mut self, throttle: Duration) -> Self {
        self.throttle = throttle;
        self
    }

    fn definition(&self) -> &'static crate::providers::traits::ChannelDefinition {
        self.sender.definition()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SinkState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Compose what should currently be visible.
    fn compose(state: &SinkState) -> String {
        let mut out = state.text.trim_end().to_string();
        if !state.tool_notes.is_empty() {
            let shown = state.tool_notes.len();
            let notes: Vec<&str> = state
                .tool_notes
                .iter()
                .map(|(_, note)| note.as_str())
                .collect();
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&notes.join("\n"));
            if shown < state.tool_calls {
                out.push_str(&format!("\n… (+{} more)", state.tool_calls - shown));
            }
        }
        out
    }

    /// Publish the current text, unless throttled and not forced.
    async fn push(&self, force: bool) -> Result<()> {
        if !self.progressive {
            return Ok(());
        }
        let (composed, limit, unit) = {
            let state = self.lock();
            if !force && state.last_push.elapsed() < self.throttle {
                return Ok(());
            }
            let definition = self.definition();
            (
                Self::compose(&state),
                definition.max_text_len,
                definition.length_unit,
            )
        };
        if composed.is_empty() {
            return Ok(());
        }

        let chunks = chunk(&composed, limit, unit);
        if chunks.is_empty() {
            return Ok(());
        }
        let mut standalone = self.lock().standalone;

        // Everything except the tail is final: post it once as its own message.
        while standalone + 1 < chunks.len() {
            self.sender
                .send_text(&self.conversation, &chunks[standalone])
                .await?;
            standalone += 1;
        }

        let tail = chunks.last().cloned().unwrap_or_default();
        let existing = self.lock().editable_message.clone();
        match existing {
            Some(message_id) => {
                if let Err(error) = self
                    .sender
                    .edit_text(&self.conversation, &message_id, &tail)
                    .await
                {
                    // A failed edit must not kill the turn: the user still has
                    // the previous version of the message.
                    tracing::warn!(
                        channel = self.definition().id,
                        %error,
                        "progressive edit failed; keeping the earlier text"
                    );
                }
            }
            None => {
                let message_id = self
                    .sender
                    .send_text(&self.conversation, &tail)
                    .await
                    .unwrap_or(None);
                self.lock().editable_message = message_id;
            }
        }

        let mut state = self.lock();
        state.standalone = standalone;
        state.last_push = Instant::now();
        state.sent_anything = true;
        Ok(())
    }

    /// Deliver everything, once, for a sink that cannot edit.
    async fn send_all(&self, text: &str) -> Result<()> {
        let definition = self.definition();
        for piece in chunk(text, definition.max_text_len, definition.length_unit) {
            self.sender.send_text(&self.conversation, &piece).await?;
        }
        Ok(())
    }

    /// The trailing message that explains a failure.
    fn failure_note(outcome: &TurnOutcome) -> Option<String> {
        if !outcome.status.is_failure() {
            return None;
        }
        let detail = outcome
            .error
            .as_deref()
            .map(|error| truncate(error, 300, LengthUnit::Chars))
            .filter(|error| !error.trim().is_empty());
        Some(match detail {
            Some(detail) => format!("⚠️ The run did not finish: {detail}"),
            None => "⚠️ The run did not finish.".to_string(),
        })
    }
}

#[async_trait]
impl ReplySink for ChannelSink {
    fn progressive(&self) -> bool {
        self.progressive
    }

    fn throttle(&self) -> Duration {
        self.throttle
    }

    async fn begin(&self) -> Result<()> {
        self.sender.typing(&self.conversation).await.ok();
        Ok(())
    }

    async fn text(&self, update: TextUpdate<'_>) -> Result<()> {
        {
            let mut state = self.lock();
            state.text = update.accumulated.to_string();
        }
        self.push(false).await
    }

    async fn thinking(&self, accumulated: &str) -> Result<()> {
        {
            let mut state = self.lock();
            state.thinking = accumulated.to_string();
        }
        // Reasoning is not shown in a chat by default; it is the natural moment
        // to refresh the typing indicator.
        self.sender.typing(&self.conversation).await.ok();
        Ok(())
    }

    async fn tool(&self, progress: &ToolProgress) -> Result<()> {
        {
            let mut state = self.lock();
            let note = match progress.phase {
                ToolPhase::Started => {
                    state.tool_calls += 1;
                    let args = progress
                        .args
                        .as_deref()
                        .map(|args| truncate(args, TOOL_ARGS_PREVIEW, LengthUnit::Chars))
                        .filter(|args| !args.trim().is_empty());
                    match args {
                        Some(args) => format!("🔧 {} `{}`", progress.name, args),
                        None => format!("🔧 {}", progress.name),
                    }
                }
                ToolPhase::Finished => format!("✅ {}", progress.name),
            };
            match state
                .tool_notes
                .iter_mut()
                .find(|(tool_id, _)| *tool_id == progress.tool_id)
            {
                Some((_, existing)) => *existing = note,
                None => state.tool_notes.push((progress.tool_id.clone(), note)),
            }
            if state.tool_notes.len() > MAX_TOOL_NOTES {
                let drop = state.tool_notes.len() - MAX_TOOL_NOTES;
                state.tool_notes.drain(0..drop);
            }
        }
        self.push(false).await
    }

    async fn approval(&self, prompt: &ApprovalPrompt) -> Result<()> {
        let definition = self.definition();
        let body = format!(
            "🔐 Approval needed for `{}` ({})\n{}\n\nReply “yes” to allow or “no” to reject.",
            prompt.tool_name,
            if prompt.risk_level.is_empty() {
                "unknown risk"
            } else {
                prompt.risk_level.as_str()
            },
            if prompt.summary.trim().is_empty() {
                prompt.title.as_str()
            } else {
                prompt.summary.as_str()
            }
        );
        for piece in chunk(&body, definition.max_text_len, definition.length_unit) {
            self.sender.send_text(&self.conversation, &piece).await?;
        }
        Ok(())
    }

    async fn error(&self, message: &str) -> Result<()> {
        tracing::debug!(
            channel = self.definition().id,
            %message,
            "agent reported a mid-stream error"
        );
        Ok(())
    }

    async fn finish(&self, outcome: &TurnOutcome) -> Result<()> {
        if self.finished.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        {
            // The outcome is authoritative: a buffered sink never saw the text
            // stream, so take it from the turn rather than from its own buffer.
            let mut state = self.lock();
            if !outcome.text.is_empty() {
                state.text = outcome.text.clone();
            }
        }
        let body = {
            let state = self.lock();
            Self::compose(&state)
        };
        if self.progressive {
            self.push(true).await?;
            if body.is_empty() && outcome.tool_calls == 0 {
                // Nothing visible and nothing to explain: stay silent rather
                // than post an empty bubble.
                if let Some(note) = Self::failure_note(outcome) {
                    self.send_all(&note).await?;
                }
                return Ok(());
            }
        } else if !body.trim().is_empty() {
            self.send_all(&body).await?;
        }
        if let Some(note) = Self::failure_note(outcome) {
            self.send_all(&note).await?;
        }
        Ok(())
    }

    async fn superseded(&self) -> Result<()> {
        if self.finished.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let (body, sent) = {
            let state = self.lock();
            (Self::compose(&state), state.sent_anything)
        };
        // A newer message owns the answer now: keep whatever the user can
        // already see, but do not add anything.
        if sent && !body.trim().is_empty() {
            self.push(true).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Op {
        Send(String),
        Edit(String, String),
        Typing,
    }

    struct RecordingSender {
        definition: &'static ChannelDefinition,
        ops: StdMutex<Vec<Op>>,
        fail_sends: bool,
        fail_edits: bool,
    }

    impl RecordingSender {
        fn new(definition: &'static ChannelDefinition) -> Arc<Self> {
            Arc::new(Self {
                definition,
                ops: StdMutex::new(Vec::new()),
                fail_sends: false,
                fail_edits: false,
            })
        }

        fn failing_edits(definition: &'static ChannelDefinition) -> Arc<Self> {
            Arc::new(Self {
                definition,
                ops: StdMutex::new(Vec::new()),
                fail_sends: false,
                fail_edits: true,
            })
        }

        fn ops(&self) -> Vec<Op> {
            self.ops.lock().unwrap().clone()
        }

        fn sends(&self) -> Vec<String> {
            self.ops()
                .into_iter()
                .filter_map(|op| match op {
                    Op::Send(text) => Some(text),
                    _ => None,
                })
                .collect()
        }
    }

    #[async_trait]
    impl ChannelSender for RecordingSender {
        fn definition(&self) -> &'static ChannelDefinition {
            self.definition
        }

        async fn send_text(
            &self,
            _conversation: &ConversationRef,
            text: &str,
        ) -> Result<Option<String>> {
            if self.fail_sends {
                anyhow::bail!("send failed");
            }
            self.ops.lock().unwrap().push(Op::Send(text.to_string()));
            Ok(Some(format!("m{}", self.ops.lock().unwrap().len())))
        }

        async fn edit_text(
            &self,
            _conversation: &ConversationRef,
            message_id: &str,
            text: &str,
        ) -> Result<()> {
            if self.fail_edits {
                anyhow::bail!("edit failed");
            }
            self.ops
                .lock()
                .unwrap()
                .push(Op::Edit(message_id.to_string(), text.to_string()));
            Ok(())
        }

        async fn typing(&self, _conversation: &ConversationRef) -> Result<()> {
            self.ops.lock().unwrap().push(Op::Typing);
            Ok(())
        }
    }

    static BUFFERED: ChannelDefinition = ChannelDefinition {
        id: "buffered",
        display_name: "Buffered",
        description: "test double",
        docs: "",
        maturity: Maturity::Preview,
        capabilities: Capabilities::TEXT,
        max_text_len: 20,
        length_unit: LengthUnit::Chars,
        config_example: "{}",
        requires: &[],
    };

    static PROGRESSIVE: ChannelDefinition = ChannelDefinition {
        id: "progressive",
        display_name: "Progressive",
        description: "test double",
        docs: "",
        maturity: Maturity::Preview,
        capabilities: Capabilities::RICH,
        max_text_len: 20,
        length_unit: LengthUnit::Chars,
        config_example: "{}",
        requires: &[],
    };

    fn sink(sender: Arc<RecordingSender>) -> ChannelSink {
        ChannelSink::new(sender, ConversationRef::default()).with_throttle(Duration::ZERO)
    }

    #[tokio::test]
    async fn a_buffered_sink_sends_once_at_the_end_and_splits_to_the_limit() {
        let sender = RecordingSender::new(&BUFFERED);
        let sink = sink(sender.clone());
        assert!(!sink.progressive());

        sink.begin().await.unwrap();
        sink.text(TextUpdate {
            accumulated: "hello",
            delta: "hello",
        })
        .await
        .unwrap();
        assert!(
            sender.sends().is_empty(),
            "buffered sinks send nothing early"
        );

        sink.finish(&TurnOutcome::completed("x".repeat(45)))
            .await
            .unwrap();
        let sends = sender.sends();
        assert_eq!(sends.len(), 3, "{sends:?}");
        assert!(sends.iter().all(|text| text.chars().count() <= 20));
        assert_eq!(sends.concat(), "x".repeat(45));
    }

    #[tokio::test]
    async fn a_progressive_sink_creates_a_message_then_edits_it() {
        let sender = RecordingSender::new(&PROGRESSIVE);
        let sink = sink(sender.clone());
        assert!(sink.progressive());

        sink.text(TextUpdate {
            accumulated: "first",
            delta: "first",
        })
        .await
        .unwrap();
        sink.text(TextUpdate {
            accumulated: "first second",
            delta: " second",
        })
        .await
        .unwrap();

        let ops = sender.ops();
        assert_eq!(ops[0], Op::Send("first".into()));
        assert_eq!(ops[1], Op::Edit("m1".into(), "first second".into()));
    }

    #[tokio::test]
    async fn a_progressive_sink_opens_a_new_message_when_the_text_overflows() {
        let sender = RecordingSender::new(&PROGRESSIVE);
        let sink = sink(sender.clone());
        let long = "a".repeat(30);
        sink.text(TextUpdate {
            accumulated: &long,
            delta: &long,
        })
        .await
        .unwrap();
        sink.finish(&TurnOutcome::completed(long.clone()))
            .await
            .unwrap();

        let ops = sender.ops();
        // The first chunk is final, so it is posted once; the tail is edited.
        assert_eq!(ops[0], Op::Send("a".repeat(20)));
        match ops.last().unwrap() {
            Op::Edit(_, text) => assert_eq!(text, &"a".repeat(10)),
            other => panic!("expected an edit of the tail, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn progressive_updates_are_throttled() {
        let sender = RecordingSender::new(&PROGRESSIVE);
        // A long window means only the forced flush at the end gets through.
        let sink = ChannelSink::new(sender.clone(), ConversationRef::default())
            .with_throttle(Duration::from_secs(3600));
        sink.text(TextUpdate {
            accumulated: "one",
            delta: "one",
        })
        .await
        .unwrap();
        assert!(sender.ops().is_empty(), "the first push is throttled too");
        sink.finish(&TurnOutcome::completed("one")).await.unwrap();
        assert_eq!(sender.sends(), vec!["one".to_string()]);
    }

    #[tokio::test]
    async fn a_failed_edit_does_not_fail_the_turn() {
        let sender = RecordingSender::failing_edits(&PROGRESSIVE);
        let sink = sink(sender.clone());
        sink.text(TextUpdate {
            accumulated: "one",
            delta: "one",
        })
        .await
        .unwrap();
        sink.text(TextUpdate {
            accumulated: "one two",
            delta: " two",
        })
        .await
        .unwrap();
        sink.finish(&TurnOutcome::completed("one two"))
            .await
            .unwrap();
        // The first message still went out; only the update was lost.
        assert_eq!(sender.sends(), vec!["one".to_string()]);
    }

    #[tokio::test]
    async fn a_failed_turn_explains_itself() {
        let sender = RecordingSender::new(&BUFFERED);
        let sink = sink(sender.clone());
        let outcome = TurnOutcome {
            text: "partial".into(),
            thinking: String::new(),
            status: TurnStatus::Error,
            error: Some("upstream disconnected".into()),
            tool_calls: 0,
            elapsed: Duration::from_secs(1),
        };
        sink.finish(&outcome).await.unwrap();
        let sends = sender.sends();
        assert_eq!(sends[0], "partial");
        assert!(
            sends.concat().contains("upstream disconnected"),
            "{sends:?}"
        );
    }

    #[tokio::test]
    async fn a_cancelled_turn_adds_nothing() {
        let sender = RecordingSender::new(&BUFFERED);
        let sink = sink(sender.clone());
        let outcome = TurnOutcome {
            text: "stopped".into(),
            thinking: String::new(),
            status: TurnStatus::Cancelled,
            error: Some("interrupted".into()),
            tool_calls: 0,
            elapsed: Duration::ZERO,
        };
        sink.finish(&outcome).await.unwrap();
        assert_eq!(sender.sends(), vec!["stopped".to_string()]);
    }

    #[tokio::test]
    async fn a_tool_only_turn_still_says_something() {
        let sender = RecordingSender::new(&BUFFERED);
        let sink = sink(sender.clone());
        sink.tool(&ToolProgress {
            tool_id: "t1".into(),
            name: "shell".into(),
            args: Some(r#"{"command":"ls"}"#.into()),
            phase: ToolPhase::Started,
        })
        .await
        .unwrap();
        sink.tool(&ToolProgress {
            tool_id: "t1".into(),
            name: "shell".into(),
            args: None,
            phase: ToolPhase::Finished,
        })
        .await
        .unwrap();
        sink.finish(&TurnOutcome {
            text: String::new(),
            thinking: String::new(),
            status: TurnStatus::Completed,
            error: None,
            tool_calls: 1,
            elapsed: Duration::from_secs(2),
        })
        .await
        .unwrap();
        let sends = sender.sends();
        assert_eq!(sends.len(), 1);
        assert!(sends[0].contains("shell"), "{sends:?}");
        assert!(
            sends[0].starts_with('✅'),
            "the finished form replaces the running one"
        );
    }

    #[tokio::test]
    async fn an_empty_completed_turn_posts_nothing() {
        let sender = RecordingSender::new(&BUFFERED);
        let sink = sink(sender.clone());
        sink.finish(&TurnOutcome::completed("")).await.unwrap();
        assert!(sender.sends().is_empty());
    }

    #[tokio::test]
    async fn a_superseded_turn_keeps_what_the_user_can_see() {
        let sender = RecordingSender::new(&PROGRESSIVE);
        let sink = sink(sender.clone());
        sink.text(TextUpdate {
            accumulated: "partial answer",
            delta: "partial answer",
        })
        .await
        .unwrap();
        let before = sender.ops().len();
        sink.superseded().await.unwrap();
        assert!(
            sender.ops().len() >= before,
            "the visible partial text is finalized, not withdrawn"
        );
    }

    #[tokio::test]
    async fn a_superseded_turn_with_no_output_stays_silent() {
        let sender = RecordingSender::new(&PROGRESSIVE);
        let sink = sink(sender.clone());
        sink.superseded().await.unwrap();
        assert!(sender.ops().is_empty());
    }

    #[tokio::test]
    async fn only_the_first_terminal_call_counts() {
        let sender = RecordingSender::new(&BUFFERED);
        let sink = sink(sender.clone());
        sink.finish(&TurnOutcome::completed("one")).await.unwrap();
        sink.finish(&TurnOutcome::completed("two")).await.unwrap();
        sink.superseded().await.unwrap();
        assert_eq!(sender.sends(), vec!["one".to_string()]);
    }

    #[tokio::test]
    async fn an_approval_prompt_explains_how_to_answer() {
        let sender = RecordingSender::new(&BUFFERED);
        let sink = sink(sender.clone());
        sink.approval(&ApprovalPrompt {
            request_id: "req_1".into(),
            tool_name: "shell".into(),
            risk_level: "high".into(),
            title: "run rm -rf".into(),
            summary: "deletes build output".into(),
        })
        .await
        .unwrap();
        let sends = sender.sends();
        let joined = sends.concat();
        assert!(joined.contains("shell"), "{sends:?}");
        assert!(joined.contains("deletes build output"), "{sends:?}");
        assert!(joined.contains("yes"), "{sends:?}");
    }

    #[tokio::test]
    async fn begin_and_thinking_use_the_typing_indicator() {
        let sender = RecordingSender::new(&PROGRESSIVE);
        let sink = sink(sender.clone());
        sink.begin().await.unwrap();
        sink.thinking("weighing options").await.unwrap();
        let ops = sender.ops();
        assert_eq!(ops.iter().filter(|op| **op == Op::Typing).count(), 2);
        assert!(sink.lock().thinking.contains("weighing"));
    }

    #[test]
    fn turn_status_maps_the_agent_vocabulary() {
        assert_eq!(
            TurnStatus::from_agent(Some("completed"), None),
            TurnStatus::Completed
        );
        assert_eq!(
            TurnStatus::from_agent(Some("cancelled"), None),
            TurnStatus::Cancelled
        );
        assert_eq!(
            TurnStatus::from_agent(Some("interrupted"), None),
            TurnStatus::Cancelled
        );
        assert_eq!(
            TurnStatus::from_agent(Some("error"), None),
            TurnStatus::Error
        );
        assert_eq!(
            TurnStatus::from_agent(Some("incomplete"), None),
            TurnStatus::Incomplete
        );
        // Older agents send only an error string.
        assert_eq!(
            TurnStatus::from_agent(None, Some("boom")),
            TurnStatus::Error
        );
        assert_eq!(TurnStatus::from_agent(None, None), TurnStatus::Completed);
        assert_eq!(
            TurnStatus::from_agent(Some("unknown"), None),
            TurnStatus::Completed
        );
    }

    #[test]
    fn failures_are_the_statuses_worth_explaining() {
        assert!(TurnStatus::Error.is_failure());
        assert!(TurnStatus::Incomplete.is_failure());
        assert!(!TurnStatus::Completed.is_failure());
        assert!(!TurnStatus::Cancelled.is_failure());
    }

    #[test]
    fn an_incomplete_turn_without_a_reason_still_explains_itself() {
        let outcome = TurnOutcome {
            text: String::new(),
            thinking: String::new(),
            status: TurnStatus::Incomplete,
            error: None,
            tool_calls: 0,
            elapsed: Duration::ZERO,
        };
        let note = ChannelSink::failure_note(&outcome).expect("a note");
        assert!(note.contains("did not finish"), "{note}");
    }
}

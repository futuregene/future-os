//! The CLI channel: talk to the agent from the same terminal that runs the
//! bridge.
//!
//! Small, but not a toy: it is the fastest way to exercise the whole pipeline
//! (policy, sessions, queueing, streaming, approval routing) without a
//! platform, and the reference for how thin a provider can be.

use anyhow::Result;
use async_trait::async_trait;
use std::io::{BufRead, Write};
use std::sync::Arc;

use crate::bridge::{ChatKind, ConversationRef, Inbound, ProviderCtx, SenderRef};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "cli",
    display_name: "Terminal",
    description: "Read prompts from stdin and stream answers to stdout.",
    docs: "docs/guide/channels-cli.md",
    maturity: Maturity::Live,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: true,
        reactions: false,
        media_in: false,
        media_out: false,
        mention_gate: false,
    },
    max_text_len: 100_000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true
}"#,
    requires: &[],
};

/// The conversation every terminal prompt belongs to.
const CONVERSATION: &str = "cli";

/// Lines that end the session.
const EXIT_COMMANDS: [&str; 2] = ["/quit", "/exit"];

/// Writes replies to stdout.
pub struct StdoutSender;

#[async_trait]
impl ChannelSender for StdoutSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        _conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let mut stdout = std::io::stdout();
        writeln!(stdout, "{text}")?;
        stdout.flush()?;
        Ok(None)
    }

    async fn typing(&self, _conversation: &ConversationRef) -> Result<()> {
        let mut stdout = std::io::stdout();
        write!(stdout, "\r⏳ working…")?;
        stdout.flush()?;
        Ok(())
    }
}

/// Read lines from stdin.
struct Cli;

/// Read `reader` line by line into `tx` until it ends, the receiver closes, or a
/// read fails.
///
/// Blocking by design: this runs on a blocking thread so the async side stays
/// free to run turns.
fn pump_lines<R: BufRead>(mut reader: R, tx: tokio::sync::mpsc::Sender<String>) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {
                if tx.blocking_send(std::mem::take(&mut line)).is_err() {
                    return;
                }
            }
            Err(error) => {
                tracing::warn!(%error, "cannot read a terminal prompt");
                return;
            }
        }
    }
}

impl Cli {
    /// Serve prompts from stdin until it ends or the user asks to quit.
    async fn serve_stdin(&self, ctx: &ProviderCtx, sender: Arc<dyn ChannelSender>) -> Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        let reader = tokio::task::spawn_blocking(move || {
            // The lock is taken inside the blocking task, so nothing
            // `Send`-hostile crosses a thread boundary.
            pump_lines(std::io::stdin().lock(), tx);
        });
        let served = self.drive(ctx, sender, &mut rx).await;
        reader.abort();
        served
    }

    /// Consume prompts until the reader ends or the user asks to quit.
    ///
    /// Split from the stdin plumbing so the loop is testable: driving a real
    /// stdin would make a test block on whatever the terminal sends.
    async fn drive(
        &self,
        ctx: &ProviderCtx,
        sender: Arc<dyn ChannelSender>,
        rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<()> {
        while let Some(line) = rx.recv().await {
            let text = line.trim_end_matches(['\n', '\r']).trim().to_string();
            if text.is_empty() {
                continue;
            }
            if EXIT_COMMANDS.contains(&text.as_str()) {
                break;
            }
            let inbound = Inbound {
                message_id: format!("cli-{}", crate::bridge::dedup::now_ms()),
                sender: SenderRef {
                    id: "terminal".into(),
                    display: Some("terminal".into()),
                },
                conversation: ConversationRef {
                    id: CONVERSATION.into(),
                    thread_id: None,
                    kind: ChatKind::Direct,
                },
                text,
                media: Vec::new(),
                addressed_to_bot: true,
                created_at_ms: Some(crate::bridge::dedup::now_ms()),
                raw: None,
            };
            let outcome = ctx.handle(inbound, sender.clone()).await;
            tracing::debug!(?outcome, "terminal prompt handled");
        }
        Ok(())
    }
}

#[async_trait]
impl Provider for Cli {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, _ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        Ok(Arc::new(StdoutSender))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let sender = self.sender(&ctx)?;
        ctx.mark_running();
        self.serve_stdin(&ctx, sender).await
    }

    async fn probe(&self, _ctx: &ProviderCtx) -> Result<String> {
        Ok("terminal channel is always available".to_string())
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Cli)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{HandleOutcome, MediaKind, MediaRef};
    use crate::config::AgentConfig;
    use std::sync::Mutex as StdMutex;

    /// A sender that records text instead of writing to the test's stdout.
    #[derive(Default)]
    struct RecordingSender {
        sent: StdMutex<Vec<String>>,
    }

    impl RecordingSender {
        fn sent(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChannelSender for RecordingSender {
        fn definition(&self) -> &'static ChannelDefinition {
            &DEFINITION
        }

        async fn send_text(
            &self,
            _conversation: &ConversationRef,
            text: &str,
        ) -> Result<Option<String>> {
            self.sent.lock().unwrap().push(text.to_string());
            Ok(None)
        }
    }

    fn ctx(label: &str) -> ProviderCtx {
        let data_dir = crate::test_support::temp_dir(label);
        let sessions = Arc::new(crate::session_store::SessionStore::new(
            data_dir.join("sessions.json"),
        ));
        let bridge = crate::bridge::Bridge::new(
            Arc::new(AgentConfig {
                grpc_addr: "http://127.0.0.1:1".into(),
                cwd: data_dir.to_string_lossy().into_owned(),
                ..AgentConfig::default()
            }),
            crate::policy::AccessPolicyConfig {
                dm_policy: "open".into(),
                ..Default::default()
            },
            data_dir.clone(),
            Arc::new(crate::status::StatusBoard::new(
                data_dir.join("status.json"),
            )),
        );
        ProviderCtx::new(
            &DEFINITION,
            serde_json::json!({"enabled": true}),
            bridge,
            data_dir,
            sessions,
            Arc::new(tokio::sync::Notify::new()),
        )
    }

    #[tokio::test]
    async fn the_stdout_sender_writes_and_reports_no_message_id() {
        let sender = StdoutSender;
        assert_eq!(sender.definition().id, "cli");
        assert!(sender
            .send_text(&ConversationRef::default(), "hello")
            .await
            .unwrap()
            .is_none());
        assert!(sender.typing(&ConversationRef::default()).await.is_ok());
    }

    #[test]
    fn the_definition_is_complete_enough_to_start() {
        assert!(DEFINITION.is_implemented());
        assert!(DEFINITION.capabilities.receive && DEFINITION.capabilities.send);
        assert!(!DEFINITION.capabilities.mention_gate);
        assert_eq!(DEFINITION.maturity, Maturity::Live);
    }

    #[test]
    fn a_terminal_prompt_maps_onto_a_direct_conversation() {
        let inbound = Inbound::new_direct("m1", "terminal", CONVERSATION, "hello");
        assert_eq!(inbound.conversation.kind, ChatKind::Direct);
        assert!(inbound.media.is_empty());
        assert_eq!(MediaRef::default().kind, MediaKind::Unknown);
    }

    #[tokio::test]
    async fn probing_reports_availability_without_an_agent() {
        let summary = Cli
            .probe(&ProviderCtx::offline(&DEFINITION))
            .await
            .expect("probe");
        assert!(summary.contains("available"), "{summary}");
    }

    #[tokio::test]
    async fn the_sender_is_built_even_without_a_platform() {
        let sender = Cli
            .sender(&ProviderCtx::offline(&DEFINITION))
            .expect("sender");
        assert_eq!(sender.definition().id, "cli");
    }

    #[tokio::test]
    async fn a_line_becomes_a_prompt_and_a_quit_line_ends_the_session() {
        let ctx = ctx("cli-serve");
        let sender = Arc::new(RecordingSender::default());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        // The pump reads exactly as stdin would: one line per message.
        let pump = tokio::task::spawn_blocking(move || {
            pump_lines(
                std::io::Cursor::new(b"hello there\n\n/quit\nignored\n".to_vec()),
                tx,
            );
        });
        Cli.drive(&ctx, sender.clone(), &mut rx)
            .await
            .expect("drive");
        pump.abort();
        // The prompt reached the bridge (which reports the agent as unreachable
        // and says so on the terminal), and the loop stopped at /quit so the
        // trailing line was never submitted.
        let sent = sender.sent();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("Cannot reach the agent"), "{sent:?}");
        assert_eq!(ctx.sessions().get("cli:cli", None), None);
    }

    #[test]
    fn the_pump_forwards_lines_and_stops_at_end_of_input() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        pump_lines(std::io::Cursor::new(b"one\ntwo\n".to_vec()), tx);
        assert_eq!(rx.try_recv().unwrap(), "one\n");
        assert_eq!(rx.try_recv().unwrap(), "two\n");
        assert!(rx.try_recv().is_err(), "the pump ended at EOF");
    }

    #[test]
    fn the_pump_stops_when_the_consumer_goes_away() {
        // Closing the receiver must end the reader rather than block forever.
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(1);
        drop(rx);
        pump_lines(std::io::Cursor::new(b"one\ntwo\n".to_vec()), tx);
    }

    #[tokio::test]
    async fn an_input_that_ends_immediately_finishes_the_session() {
        let ctx = ctx("cli-serve-eof");
        let sender = Arc::new(RecordingSender::default());
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);
        drop(_tx);
        Cli.drive(&ctx, sender.clone(), &mut rx)
            .await
            .expect("drive");
        assert!(sender.sent().is_empty());
    }

    #[tokio::test]
    async fn the_exit_commands_are_both_honoured() {
        for command in EXIT_COMMANDS {
            let ctx = ctx("cli-serve-exit");
            let sender = Arc::new(RecordingSender::default());
            let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
            tx.try_send(format!("{command}\n")).unwrap();
            Cli.drive(&ctx, sender.clone(), &mut rx)
                .await
                .expect("drive");
            assert!(sender.sent().is_empty(), "{command} must not be a prompt");
        }
    }

    #[tokio::test]
    async fn a_denied_message_does_not_reach_the_agent() {
        // The default policy is a DM allowlist with no entries, so a terminal
        // prompt is refused unless the user opts in — a safe default.
        let data_dir = crate::test_support::temp_dir("cli-denied");
        let bridge = crate::bridge::Bridge::new(
            Arc::new(AgentConfig::default()),
            crate::policy::AccessPolicyConfig::default(),
            data_dir.clone(),
            Arc::new(crate::status::StatusBoard::new(
                data_dir.join("status.json"),
            )),
        );
        let ctx = ProviderCtx::new(
            &DEFINITION,
            serde_json::json!({"enabled": true}),
            bridge,
            data_dir.clone(),
            Arc::new(crate::session_store::SessionStore::new(
                data_dir.join("sessions.json"),
            )),
            Arc::new(tokio::sync::Notify::new()),
        );
        let outcome = ctx
            .handle(
                Inbound::new_direct("m1", "terminal", CONVERSATION, "hi"),
                Arc::new(StdoutSender),
            )
            .await;
        assert!(matches!(outcome, HandleOutcome::Denied(_)), "{outcome:?}");
    }
}

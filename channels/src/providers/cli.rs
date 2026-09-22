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

/// Reads lines from stdin.
struct Cli;

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
        // Reading stdin blocks, so it runs on a blocking thread; the loop ends
        // when stdin closes (EOF) or the process is asked to stop.
        let mut line = String::new();
        loop {
            line.clear();
            let read = tokio::task::block_in_place(|| {
                let stdin = std::io::stdin();
                stdin.lock().read_line(&mut line)
            });
            match read {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error) => return Err(error.into()),
            }
            let text = line.trim_end_matches(['\n', '\r']).trim().to_string();
            if text.is_empty() {
                continue;
            }
            if text == "/quit" || text == "/exit" {
                return Ok(());
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
    use crate::bridge::{HandleOutcome, MediaRef};

    #[tokio::test]
    async fn the_sender_writes_exactly_the_text_it_was_given() {
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
    }

    #[test]
    fn a_terminal_prompt_maps_onto_a_direct_conversation() {
        let inbound = Inbound::new_direct("m1", "terminal", CONVERSATION, "hello");
        assert_eq!(inbound.conversation.kind, ChatKind::Direct);
        assert!(inbound.media.is_empty());
        assert!(matches!(
            MediaRef::default().kind,
            crate::bridge::MediaKind::Unknown
        ));
    }

    #[tokio::test]
    async fn probing_reports_availability_without_an_agent() {
        let ctx = crate::bridge::ProviderCtx::offline(&DEFINITION);
        let summary = Cli.probe(&ctx).await.unwrap();
        assert!(summary.contains("available"), "{summary}");
    }

    #[tokio::test]
    async fn a_denied_message_does_not_reach_the_agent() {
        let ctx = crate::bridge::ProviderCtx::offline(&DEFINITION);
        let outcome = ctx
            .handle(
                Inbound::new_direct("m1", "terminal", CONVERSATION, "hi"),
                Arc::new(StdoutSender),
            )
            .await;
        // Default policy is a DM allowlist with no entries, so nothing is
        // accepted — a safe default rather than an open door.
        assert!(matches!(outcome, HandleOutcome::Denied(_)), "{outcome:?}");
    }
}

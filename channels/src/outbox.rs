//! Running the durable outbound queue.
//!
//! [`crate::delivery`] decides *what* to retry; this module does it. Delivery is
//! built from configuration at the moment of sending — not from a running
//! channel — so a message can go out while the bridge is starting, while a
//! channel is reconnecting, or from a one-shot CLI command with no bridge at all.

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::Arc;

use crate::bridge::{Bridge, ConversationRef, ProviderCtx};
use crate::config::{AgentConfig, ChannelConfig};
use crate::delivery::{DeliveryQueue, DeliveryState};
use crate::policy::AccessPolicyConfig;
use crate::providers::traits::{ChannelSender, Provider};
use crate::providers::{definition, registry};
use crate::session_store::SessionStore;
use crate::status::StatusBoard;

/// Outcome of one drain pass.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DrainSummary {
    /// Delivered.
    pub sent: usize,
    /// Failed now, will be retried later.
    pub retried: usize,
    /// Given up on (permanent failure, or attempts exhausted).
    pub failed: usize,
    /// Nothing to do.
    pub skipped: usize,
}

impl DrainSummary {
    pub fn touched(&self) -> usize {
        self.sent + self.retried + self.failed
    }
}

/// Delivers messages from the durable queue.
pub struct Outbox {
    queue: Arc<DeliveryQueue>,
    config: ChannelConfig,
    agent_cfg: Arc<AgentConfig>,
    root: PathBuf,
    status: Arc<StatusBoard>,
}

impl Outbox {
    pub fn new(
        queue: Arc<DeliveryQueue>,
        config: ChannelConfig,
        agent_cfg: Arc<AgentConfig>,
        root: PathBuf,
        status: Arc<StatusBoard>,
    ) -> Self {
        Self {
            queue,
            config,
            agent_cfg,
            root,
            status,
        }
    }

    pub fn queue(&self) -> &DeliveryQueue {
        &self.queue
    }

    /// Queue a message for delivery, then try it once.
    pub async fn enqueue(
        &self,
        channel: &str,
        conversation: ConversationRef,
        text: &str,
    ) -> Result<String> {
        let id = self.queue.enqueue(channel, conversation, text)?;
        self.drain_due().await;
        Ok(id)
    }

    /// Send now, reporting a failure to the caller instead of queueing it.
    pub async fn deliver_now(
        &self,
        channel: &str,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<()> {
        let sender = self.sender(channel)?;
        let definition = sender.definition();
        let pieces = crate::transport::chunk(text, definition.max_text_len, definition.length_unit);
        if pieces.is_empty() {
            return Err(anyhow!("refusing to send an empty message"));
        }
        for piece in pieces {
            sender.send_text(conversation, &piece).await?;
        }
        self.status
            .count_outbound(channel, crate::status::now_unix());
        Ok(())
    }

    /// Try every message whose backoff has elapsed.
    pub async fn drain_due(&self) -> DrainSummary {
        let mut summary = DrainSummary::default();
        for entry in self.queue.due(crate::bridge::dedup::now_ms()) {
            let sender = match self.sender(&entry.channel) {
                Ok(sender) => sender,
                Err(error) => {
                    // The channel cannot be built from this configuration: the
                    // message may still go out after a restart fixes it, so it
                    // keeps its place in the queue until attempts run out.
                    let reason = format!("{} is not available: {error}", entry.channel);
                    tracing::warn!(channel = %entry.channel, %error, "cannot deliver a queued message yet");
                    match self.queue.record_failure(&entry.id, &reason) {
                        DeliveryState::Failed => summary.failed += 1,
                        _ => summary.retried += 1,
                    }
                    continue;
                }
            };
            let definition = sender.definition();
            let pieces = crate::transport::chunk(
                &entry.text,
                definition.max_text_len,
                definition.length_unit,
            );
            let mut failure: Option<String> = None;
            for piece in pieces {
                if let Err(error) = sender.send_text(&entry.conversation, &piece).await {
                    failure = Some(error.to_string());
                    break;
                }
            }
            match failure {
                None => {
                    let _ = self.queue.record_success(&entry.id);
                    self.status
                        .count_outbound(&entry.channel, crate::status::now_unix());
                    summary.sent += 1;
                }
                Some(error) => {
                    let state = self.queue.record_failure(&entry.id, &error);
                    match state {
                        DeliveryState::Failed => {
                            tracing::warn!(channel = %entry.channel, %error, "giving up on a queued delivery");
                            summary.failed += 1;
                        }
                        _ => {
                            tracing::debug!(channel = %entry.channel, %error, "queued delivery will be retried");
                            summary.retried += 1;
                        }
                    }
                }
            }
        }
        summary
    }

    /// Build the outbound half of a channel from configuration.
    pub fn sender(&self, channel: &str) -> Result<Arc<dyn ChannelSender>> {
        let (provider, ctx) = self.context(channel)?;
        provider.sender(&ctx)
    }

    /// Build a channel's provider and the context it runs against.
    ///
    /// Configuration is read fresh, so a channel can be probed before the
    /// bridge that would normally host it has ever started.
    pub fn context(&self, channel: &str) -> Result<(Box<dyn Provider>, ProviderCtx)> {
        let definition = definition(channel)
            .ok_or_else(|| anyhow!("unknown channel `{channel}`; run `future channel list`"))?;
        let entry = registry::find(definition.id).ok_or_else(|| {
            anyhow!(
                "the {} channel delivers through its own bridge and is not supported here yet",
                definition.id
            )
        })?;
        if !definition.is_implemented() {
            return Err(anyhow!(
                "the {} channel is not implemented in this build",
                definition.id
            ));
        }
        let block = self.config.provider_config(definition.id).ok_or_else(|| {
            anyhow!(
                "the {} channel has no configuration; add a `providers.{}` block",
                definition.id,
                definition.id
            )
        })?;
        let data_dir = self.root.join(definition.id);
        let policy =
            serde_json::from_value::<AccessPolicyConfig>(block.clone()).unwrap_or_default();
        let bridge = Bridge::new(
            self.agent_cfg.clone(),
            policy,
            self.root.clone(),
            self.status.clone(),
        );
        let sessions = Arc::new(SessionStore::new(data_dir.join("sessions.json")));
        let ctx = ProviderCtx::new(
            definition,
            block,
            bridge,
            data_dir,
            sessions,
            Arc::new(tokio::sync::Notify::new()),
        );
        Ok(((entry.provider)(), ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    fn outbox(label: &str) -> Outbox {
        let dir = temp_dir(label);
        let queue = Arc::new(DeliveryQueue::load(dir.join("deliveries.json")));
        Outbox::new(
            queue,
            ChannelConfig::default(),
            Arc::new(AgentConfig::default()),
            dir.clone(),
            Arc::new(StatusBoard::new(dir.join("status.json"))),
        )
    }

    fn conversation() -> ConversationRef {
        ConversationRef {
            id: "c1".into(),
            thread_id: None,
            kind: crate::bridge::ChatKind::Direct,
        }
    }

    #[test]
    fn an_unknown_channel_is_named_in_the_error() {
        let outbox = outbox("outbox-unknown");
        let error = outbox.sender("nope").err().expect("must fail").to_string();
        assert!(error.contains("unknown channel"), "{error}");
        assert!(error.contains("future channel list"), "{error}");
    }

    #[test]
    fn a_channel_without_configuration_says_so() {
        let outbox = outbox("outbox-unconfigured");
        // The terminal channel is implemented, but nothing configured it.
        let error = outbox.sender("cli").err().expect("must fail").to_string();
        assert!(error.contains("no configuration"), "{error}");
    }

    #[test]
    fn a_self_bridged_channel_is_rejected_with_a_reason() {
        let outbox = outbox("outbox-native");
        let error = outbox
            .sender("feishu")
            .err()
            .expect("must fail")
            .to_string();
        assert!(error.contains("own bridge"), "{error}");
    }

    #[tokio::test]
    async fn an_empty_queue_drains_to_nothing() {
        let outbox = outbox("outbox-empty");
        assert_eq!(outbox.drain_due().await, DrainSummary::default());
    }

    #[tokio::test]
    async fn an_unbuildable_channel_keeps_its_place_in_the_queue() {
        let outbox = outbox("outbox-unbuildable");
        outbox
            .queue()
            .enqueue("telegram", conversation(), "hello")
            .unwrap();
        let summary = outbox.drain_due().await;
        assert_eq!(summary.retried, 1, "{summary:?}");
        assert_eq!(summary.failed, 0, "{summary:?}");
        let pending = outbox.queue().pending();
        assert_eq!(pending.len(), 1, "a fixable configuration must be retried");
        assert_eq!(pending[0].attempts, 1);
        assert!(
            pending[0]
                .last_error
                .as_deref()
                .is_some_and(|error| error.starts_with("telegram is not available")),
            "{:?}",
            pending[0].last_error
        );
    }

    #[tokio::test]
    async fn an_empty_message_is_refused_rather_than_sent() {
        let dir = temp_dir("outbox-empty-text");
        let mut config = ChannelConfig::default();
        config
            .providers
            .insert("cli".to_string(), serde_json::json!({ "enabled": true }));
        let queue = Arc::new(DeliveryQueue::load(dir.join("deliveries.json")));
        let outbox = Outbox::new(
            queue,
            config,
            Arc::new(AgentConfig::default()),
            dir.clone(),
            Arc::new(StatusBoard::new(dir.join("status.json"))),
        );
        let error = outbox
            .deliver_now("cli", &conversation(), "   ")
            .await
            .err()
            .expect("must fail")
            .to_string();
        assert!(error.contains("empty message"), "{error}");
    }

    #[tokio::test]
    async fn a_configured_terminal_channel_can_be_built() {
        let dir = temp_dir("outbox-cli");
        let mut config = ChannelConfig::default();
        config
            .providers
            .insert("cli".to_string(), serde_json::json!({ "enabled": true }));
        let queue = Arc::new(DeliveryQueue::load(dir.join("deliveries.json")));
        let outbox = Outbox::new(
            queue,
            config,
            Arc::new(AgentConfig::default()),
            dir.clone(),
            Arc::new(StatusBoard::new(dir.join("status.json"))),
        );
        let sender = outbox.sender("cli").expect("terminal sender");
        assert_eq!(sender.definition().id, "cli");
    }

    #[test]
    fn a_summary_counts_only_the_entries_it_touched() {
        let summary = DrainSummary {
            sent: 2,
            retried: 1,
            failed: 0,
            skipped: 5,
        };
        assert_eq!(summary.touched(), 3);
        assert_eq!(DrainSummary::default().touched(), 0);
    }
}

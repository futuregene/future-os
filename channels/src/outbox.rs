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
use crate::delivery::{DeliveryQueue, DeliveryState, QueuedDelivery};
use crate::policy::AccessPolicyConfig;
use crate::providers::traits::{ChannelSender, Provider};
use crate::providers::{definition, registry};
use crate::session_store::SessionStore;
use crate::status::{StatusBoard, StatusSnapshot};

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

    /// Count one delivery attempt.
    fn record(&mut self, outcome: DeliveryOutcome) {
        match outcome {
            DeliveryOutcome::Sent => self.sent += 1,
            DeliveryOutcome::Retried => self.retried += 1,
            DeliveryOutcome::Failed => self.failed += 1,
        }
    }
}

/// What one delivery attempt did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Sent,
    /// Failed now, may succeed later.
    Retried,
    /// Given up on: permanent failure, or attempts exhausted.
    Failed,
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
                    let outcome = self.record_failure(&entry, &reason);
                    summary.record(outcome);
                    continue;
                }
            };
            let outcome = self.deliver_entry(&entry, sender).await;
            summary.record(outcome);
        }
        summary
    }

    /// Deliver one queued entry through an already-built sender.
    ///
    /// Split from [`Self::drain_due`] so a test can hand it a sender double and
    /// exercise the success, transient-failure and permanent-failure paths
    /// without a platform.
    pub async fn deliver_entry(
        &self,
        entry: &QueuedDelivery,
        sender: Arc<dyn ChannelSender>,
    ) -> DeliveryOutcome {
        let definition = sender.definition();
        let pieces =
            crate::transport::chunk(&entry.text, definition.max_text_len, definition.length_unit);
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
                DeliveryOutcome::Sent
            }
            Some(error) => {
                tracing::debug!(channel = %entry.channel, %error, "queued delivery attempt failed");
                self.record_failure(entry, &error)
            }
        }
    }

    /// Record a failed attempt and classify it.
    fn record_failure(&self, entry: &QueuedDelivery, error: &str) -> DeliveryOutcome {
        match self.queue.record_failure(&entry.id, error) {
            DeliveryState::Failed => {
                tracing::warn!(channel = %entry.channel, %error, "giving up on a queued delivery");
                DeliveryOutcome::Failed
            }
            _ => DeliveryOutcome::Retried,
        }
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
    use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
    use crate::test_support::temp_dir;
    use crate::transport::LengthUnit;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    static DEFINED: ChannelDefinition = ChannelDefinition {
        id: "double",
        display_name: "Double",
        description: "test double",
        docs: "",
        maturity: Maturity::Preview,
        capabilities: Capabilities::TEXT,
        max_text_len: 10,
        length_unit: LengthUnit::Chars,
        config_example: r#"{"enabled": true}"#,
        requires: &[],
    };

    /// A sender whose behaviour each test picks.
    struct SenderDouble {
        fail_with: Option<String>,
        sent: StdMutex<Vec<String>>,
    }

    impl SenderDouble {
        fn working() -> Arc<Self> {
            Arc::new(Self {
                fail_with: None,
                sent: StdMutex::new(Vec::new()),
            })
        }

        fn failing(message: &str) -> Arc<Self> {
            Arc::new(Self {
                fail_with: Some(message.to_string()),
                sent: StdMutex::new(Vec::new()),
            })
        }

        fn sent(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChannelSender for SenderDouble {
        fn definition(&self) -> &'static ChannelDefinition {
            &DEFINED
        }

        async fn send_text(
            &self,
            _conversation: &ConversationRef,
            text: &str,
        ) -> Result<Option<String>> {
            match &self.fail_with {
                Some(message) => Err(anyhow!("{message}")),
                None => {
                    self.sent.lock().unwrap().push(text.to_string());
                    Ok(None)
                }
            }
        }
    }

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

    /// An outbox whose configuration enables the terminal channel.
    fn terminal_outbox(label: &str) -> Outbox {
        let dir = temp_dir(label);
        let mut config = ChannelConfig::default();
        config
            .providers
            .insert("cli".to_string(), serde_json::json!({ "enabled": true }));
        let queue = Arc::new(DeliveryQueue::load(dir.join("deliveries.json")));
        Outbox::new(
            queue,
            config,
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

    /// Queue a message and return the stored entry.
    fn queued(outbox: &Outbox, text: &str) -> QueuedDelivery {
        let id = outbox
            .queue()
            .enqueue("double", conversation(), text)
            .unwrap();
        outbox
            .queue()
            .all()
            .into_iter()
            .find(|entry| entry.id == id)
            .expect("the queued entry")
    }

    #[tokio::test]
    async fn a_delivered_entry_is_recorded_as_sent() {
        let outbox = outbox("outbox-entry-sent");
        let entry = queued(&outbox, "hello");
        let sender = SenderDouble::working();
        assert_eq!(
            outbox.deliver_entry(&entry, sender.clone()).await,
            DeliveryOutcome::Sent
        );
        assert_eq!(sender.sent(), vec!["hello".to_string()]);
        assert!(outbox.queue().pending().is_empty());
        // A successful send is counted for `future channel status`.
        outbox.status.flush().unwrap();
        let snapshot = StatusSnapshot::load(&outbox.status.path());
        assert_eq!(snapshot.channels["double"].outbound_count, 1);
    }

    #[tokio::test]
    async fn a_long_message_is_chunked_to_the_platform_limit() {
        let outbox = outbox("outbox-entry-chunked");
        // The double accepts 10 characters per message.
        let entry = queued(&outbox, &"x".repeat(25));
        let sender = SenderDouble::working();
        assert_eq!(
            outbox.deliver_entry(&entry, sender.clone()).await,
            DeliveryOutcome::Sent
        );
        let sent = sender.sent();
        assert_eq!(sent.len(), 3, "{sent:?}");
        assert!(sent.iter().all(|piece| piece.chars().count() <= 10));
        assert_eq!(sent.concat(), "x".repeat(25));
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried() {
        let outbox = outbox("outbox-entry-transient");
        let entry = queued(&outbox, "hello");
        let sender = SenderDouble::failing("gateway timeout");
        assert_eq!(
            outbox.deliver_entry(&entry, sender).await,
            DeliveryOutcome::Retried
        );
        let pending = outbox.queue().pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts, 1);
        assert_eq!(pending[0].last_error.as_deref(), Some("gateway timeout"));
    }

    #[tokio::test]
    async fn a_permanent_failure_gives_up_immediately() {
        let outbox = outbox("outbox-entry-permanent");
        let entry = queued(&outbox, "hello");
        let sender = SenderDouble::failing("chat not found");
        assert_eq!(
            outbox.deliver_entry(&entry, sender).await,
            DeliveryOutcome::Failed
        );
        assert!(outbox.queue().pending().is_empty());
    }

    #[tokio::test]
    async fn a_partial_send_is_still_a_failure() {
        // The first chunk went out and a later one did not: the entry must be
        // retried (or given up on), never marked delivered.
        struct FailsAfterFirst {
            sent: StdMutex<usize>,
        }

        #[async_trait]
        impl ChannelSender for FailsAfterFirst {
            fn definition(&self) -> &'static ChannelDefinition {
                &DEFINED
            }

            async fn send_text(
                &self,
                _conversation: &ConversationRef,
                _text: &str,
            ) -> Result<Option<String>> {
                let mut sent = self.sent.lock().unwrap();
                *sent += 1;
                if *sent > 1 {
                    anyhow::bail!("gateway timeout");
                }
                Ok(None)
            }
        }

        let outbox = outbox("outbox-entry-partial");
        let entry = queued(&outbox, &"y".repeat(15));
        let sender = Arc::new(FailsAfterFirst {
            sent: StdMutex::new(0),
        });
        assert_eq!(
            outbox.deliver_entry(&entry, sender).await,
            DeliveryOutcome::Retried
        );
        assert_eq!(outbox.queue().pending().len(), 1);
    }

    #[tokio::test]
    async fn draining_delivers_a_real_configured_channel() {
        // The terminal channel can always be built, so this exercises the
        // resolve-sender-then-deliver path end to end.
        let outbox = terminal_outbox("outbox-drain-cli");
        outbox
            .queue()
            .enqueue("cli", conversation(), "in the queue")
            .unwrap();
        let summary = outbox.drain_due().await;
        assert_eq!(summary.sent, 1, "{summary:?}");
        assert_eq!(summary.touched(), 1);
        assert!(outbox.queue().pending().is_empty());
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
        let error = outbox.sender("feishu").err().expect("must fail").to_string();
        assert!(error.contains("own bridge"), "{error}");
    }

    #[test]
    fn a_channel_this_build_cannot_run_is_rejected() {
        // Whichever channels are still planned must refuse to deliver, with a
        // reason the operator can act on.
        let outbox = outbox("outbox-planned");
        for entry in crate::providers::registry::planned() {
            let id = entry.definition.id;
            let mut config = outbox.config.clone();
            config
                .providers
                .insert(id.to_string(), serde_json::json!({ "enabled": true }));
            let configured = Outbox::new(
                outbox.queue.clone(),
                config,
                outbox.agent_cfg.clone(),
                outbox.root.clone(),
                outbox.status.clone(),
            );
            let error = configured.sender(id).err().expect("must refuse").to_string();
            assert!(error.contains("not implemented"), "{id}: {error}");
        }
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
                .is_some_and(|error| error.contains("telegram is not available")),
            "{:?}",
            pending[0].last_error
        );
    }

    #[tokio::test]
    async fn queueing_a_message_tries_it_immediately() {
        let outbox = terminal_outbox("outbox-enqueue-cli");
        let id = outbox
            .enqueue("cli", conversation(), "immediate")
            .await
            .expect("enqueue");
        let entry = outbox
            .queue()
            .all()
            .into_iter()
            .find(|entry| entry.id == id)
            .expect("entry");
        assert_eq!(entry.state, DeliveryState::Sent);
    }

    #[tokio::test]
    async fn an_empty_message_is_refused_rather_than_sent() {
        let outbox = terminal_outbox("outbox-empty-text");
        let error = outbox
            .deliver_now("cli", &conversation(), "   ")
            .await
            .expect_err("an empty message must be refused")
            .to_string();
        assert!(error.contains("empty message"), "{error}");
    }

    #[tokio::test]
    async fn a_configured_terminal_channel_can_be_built_and_delivered_through() {
        let outbox = terminal_outbox("outbox-cli");
        let sender = outbox.sender("cli").expect("terminal sender");
        assert_eq!(sender.definition().id, "cli");
        outbox
            .deliver_now("cli", &conversation(), "direct")
            .await
            .expect("direct delivery");
        outbox.status.flush().unwrap();
        let snapshot = StatusSnapshot::load(&outbox.status.path());
        assert_eq!(snapshot.channels["cli"].outbound_count, 1);
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

    #[test]
    fn every_delivery_outcome_lands_in_its_own_bucket() {
        let mut summary = DrainSummary::default();
        summary.record(DeliveryOutcome::Sent);
        summary.record(DeliveryOutcome::Retried);
        summary.record(DeliveryOutcome::Failed);
        summary.record(DeliveryOutcome::Failed);
        assert_eq!(summary.sent, 1);
        assert_eq!(summary.retried, 1);
        assert_eq!(summary.failed, 2);
        assert_eq!(summary.touched(), 4);
    }
}

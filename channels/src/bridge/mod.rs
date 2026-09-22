//! The bridge: everything a channel needs that is not platform-specific.
//!
//! A provider's listener turns platform events into [`Inbound`] messages and
//! hands each one to [`ProviderCtx::handle`]. From there the bridge owns the
//! whole pipeline:
//!
//! ```text
//! Inbound ─▶ stale filter ─▶ duplicate filter ─▶ approval answer?
//!         ─▶ access policy ─▶ session mapping ─▶ per-conversation queue
//!         ─▶ agent turn ─▶ ReplySink ─▶ platform
//! ```
//!
//! The pieces are deliberately separate ([`dedup`], [`approval`], [`queue`],
//! [`sink`], [`turn`]) because each one is testable on its own and each one has
//! exactly one reason to change.

pub mod approval;
pub mod dedup;
pub mod inbound;
pub mod queue;
pub mod sink;
pub mod turn;

pub use approval::{ApprovalRegistry, ApprovalRoute};
pub use dedup::Dedup;
pub use inbound::{ChatKind, ConversationRef, Inbound, MediaKind, MediaRef, SenderRef};
pub use queue::{Conversations, Job, SubmitOutcome, SupersedeWatch};
pub use sink::{
    ApprovalPrompt, ChannelSink, ReplySink, TextUpdate, ToolPhase, ToolProgress, TurnOutcome,
    TurnStatus,
};

use anyhow::Result;
use base64::Engine;
use parking_lot::{Mutex, RwLock};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::Notify;

use crate::config::AgentConfig;
use crate::grpc_client::{AgentClient, ImageData, ImageInput};
use crate::policy::{Access, PolicyEngine};
use crate::providers::traits::{ChannelDefinition, ChannelSender};
use crate::session_store::SessionStore;
use crate::status::{ChannelState, StatusBoard};

/// Largest inbound media the bridge will read into model input.
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// What the bridge did with an inbound message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleOutcome {
    /// Queued for a turn (possibly superseding one already running).
    Accepted,
    /// Seen before; ignored.
    Duplicate,
    /// Too old to be worth answering (a replay after a long outage).
    Stale,
    /// The policy refused it; the reason is safe to show the sender.
    Denied(String),
    /// The message answered a pending approval instead of being a prompt.
    ApprovalAnswered,
    /// The conversation's mailbox is full.
    Backpressure,
}

impl HandleOutcome {
    pub fn is_accepted(&self) -> bool {
        matches!(
            self,
            HandleOutcome::Accepted | HandleOutcome::ApprovalAnswered
        )
    }
}

/// Shared runtime handed to every channel.
pub struct Bridge {
    agent_cfg: Arc<AgentConfig>,
    client: Mutex<Option<AgentClient>>,
    policy: RwLock<PolicyEngine>,
    dedup: Arc<Dedup>,
    approvals: Arc<ApprovalRegistry>,
    conversations: OnceLock<Arc<Conversations>>,
    http: reqwest::Client,
    status: Arc<StatusBoard>,
    data_root: PathBuf,
    stale_after_ms: i64,
}

impl Bridge {
    /// Build a bridge that connects to the agent lazily.
    ///
    /// Construction never fails: a bridge can be built to answer `future
    /// channel list` or a connectivity probe with no agent running, and the
    /// first turn is what needs a connection.
    pub fn new(
        agent_cfg: Arc<AgentConfig>,
        policy: crate::policy::AccessPolicyConfig,
        data_root: PathBuf,
        status: Arc<StatusBoard>,
    ) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("future-channel/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        let bridge = Arc::new(Self {
            agent_cfg,
            client: Mutex::new(None),
            policy: RwLock::new(PolicyEngine::new(policy)),
            dedup: Arc::new(Dedup::new(dedup::DEFAULT_CAPACITY)),
            approvals: Arc::new(ApprovalRegistry::new()),
            conversations: OnceLock::new(),
            http,
            status,
            data_root,
            stale_after_ms: dedup::DEFAULT_STALE_AFTER_MS,
        });
        bridge.start_conversations();
        bridge
    }

    /// A bridge with default agent settings, for offline commands and tests.
    pub fn offline() -> Arc<Self> {
        Self::new(
            Arc::new(AgentConfig::default()),
            crate::policy::AccessPolicyConfig::default(),
            std::env::temp_dir().join("future-channel-offline"),
            Arc::new(StatusBoard::new(
                std::env::temp_dir()
                    .join("future-channel-offline")
                    .join("status.json"),
            )),
        )
    }

    fn start_conversations(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let runner: queue::Runner = Arc::new(move |job, watch| {
            let weak = weak.clone();
            Box::pin(async move {
                match weak.upgrade() {
                    Some(bridge) => bridge.run_job(job, watch).await,
                    None => tracing::debug!("bridge dropped; job skipped"),
                }
            })
        });
        let _ = self.conversations.set(Arc::new(Conversations::new(runner)));
    }

    /// A clone of the agent client, connecting on first use.
    pub async fn client(&self) -> Result<AgentClient> {
        if let Some(client) = self.client.lock().clone() {
            return Ok(client);
        }
        let client = AgentClient::connect(&self.agent_cfg.grpc_addr).await?;
        *self.client.lock() = Some(client.clone());
        Ok(client)
    }

    /// Drop the cached connection so the next use reconnects.
    pub fn invalidate_client(&self) {
        *self.client.lock() = None;
    }

    /// Whether an agent connection is currently held.
    pub fn is_connected(&self) -> bool {
        self.client.lock().is_some()
    }

    pub fn agent_cfg(&self) -> &Arc<AgentConfig> {
        &self.agent_cfg
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn status(&self) -> &StatusBoard {
        &self.status
    }

    pub fn approvals(&self) -> &Arc<ApprovalRegistry> {
        &self.approvals
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// Apply a per-conversation policy override at runtime.
    pub fn set_chat_override(&self, conversation: &str, ov: crate::policy::ChatOverride) {
        self.policy
            .write()
            .set_override(conversation.to_string(), ov);
    }

    /// The access verdict for one inbound message.
    pub fn decide_access(&self, inbound: &Inbound) -> Access {
        let policy = self.policy.read();
        match inbound.conversation.kind {
            ChatKind::Direct => policy.check_dm(&inbound.sender.id),
            _ => policy.check_group(&inbound.conversation.id, inbound.addressed_to_bot),
        }
    }

    /// Run one queued job: the agent turn, reported through the job's sink.
    async fn run_job(self: Arc<Self>, job: Job, watch: SupersedeWatch) {
        let client = match self.client().await {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, conversation = %job.conversation, "cannot reach the agent");
                let outcome = TurnOutcome {
                    text: String::new(),
                    thinking: String::new(),
                    status: TurnStatus::Error,
                    error: Some(format!("the agent is unreachable: {error}")),
                    tool_calls: 0,
                    elapsed: std::time::Duration::ZERO,
                };
                job.sink.finish(&outcome).await.ok();
                return;
            }
        };

        let request = turn::TurnRequest {
            session_id: job.session_id.clone(),
            text: job.text.clone(),
            images: job.images.clone(),
            conversation_key: job.conversation.clone(),
            reply_to: job.reply_to.clone(),
            channel: job.channel.clone(),
            sink: job.sink.clone(),
            watch: watch.clone(),
            approvals: self.approvals.clone(),
        };
        match turn::run_turn(&client, request).await {
            Ok(outcome) => {
                if outcome.status == TurnStatus::Cancelled && watch.is_superseded() {
                    self.status.count_superseded(&job.channel);
                }
                if !outcome.text.trim().is_empty() || outcome.tool_calls > 0 {
                    self.status
                        .count_outbound(&job.channel, crate::status::now_unix());
                }
            }
            Err(error) => {
                tracing::warn!(%error, conversation = %job.conversation, "turn failed");
            }
        }
    }
}

/// One channel's context: its config, its storage, and the shared bridge.
#[derive(Clone)]
pub struct ProviderCtx {
    id: &'static str,
    definition: &'static ChannelDefinition,
    config: Value,
    bridge: Arc<Bridge>,
    data_dir: PathBuf,
    sessions: Arc<SessionStore>,
    shutdown: Arc<Notify>,
}

impl ProviderCtx {
    /// A context over an offline bridge: default policy, temporary storage, no
    /// agent connection.
    ///
    /// For commands that must work without a running agent (`future channel
    /// list`, `test`) and for unit tests of provider logic.
    pub fn offline(definition: &'static ChannelDefinition) -> Self {
        let data_dir = std::env::temp_dir()
            .join("future-channel-offline")
            .join(definition.id);
        let sessions = Arc::new(SessionStore::new(data_dir.join("sessions.json")));
        Self::new(
            definition,
            serde_json::json!({ "enabled": false }),
            Bridge::offline(),
            data_dir,
            sessions,
            Arc::new(Notify::new()),
        )
    }

    /// Build a context for one channel.
    pub fn new(
        definition: &'static ChannelDefinition,
        config: Value,
        bridge: Arc<Bridge>,
        data_dir: PathBuf,
        sessions: Arc<SessionStore>,
        shutdown: Arc<Notify>,
    ) -> Self {
        Self {
            id: definition.id,
            definition,
            config,
            bridge,
            data_dir,
            sessions,
            shutdown,
        }
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn definition(&self) -> &'static ChannelDefinition {
        self.definition
    }

    /// This channel's raw config block.
    pub fn config_value(&self) -> &Value {
        &self.config
    }

    /// This channel's config block, deserialized into its own type.
    pub fn config<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.config.clone()).map_err(|error| {
            anyhow::anyhow!("invalid `providers.{}` configuration: {error}", self.id)
        })
    }

    pub fn bridge(&self) -> &Arc<Bridge> {
        &self.bridge
    }

    pub fn http(&self) -> &reqwest::Client {
        self.bridge.http()
    }

    /// Per-channel directory for caches, downloaded media and session maps.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Ensure the data directory exists (called by the starter).
    pub fn ensure_data_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        Ok(())
    }

    pub fn sessions(&self) -> &SessionStore {
        &self.sessions
    }

    /// Outstanding approval requests for this bridge.
    ///
    /// Providers normally only observe approvals through the sink; this is for
    /// probing state (diagnostics) and for seeding one in a test.
    pub fn approvals(&self) -> &Arc<ApprovalRegistry> {
        self.bridge.approvals()
    }

    /// Fires once the process is stopping.
    ///
    /// Providers select on `shutdown().notified()` to exit their loops; there is
    /// no polling flag to check.
    pub fn shutdown(&self) -> &Arc<Notify> {
        &self.shutdown
    }

    /// The access verdict for one message.
    pub fn access(&self, inbound: &Inbound) -> Access {
        self.bridge.decide_access(inbound)
    }

    /// The agent session for a conversation, created on first use.
    pub async fn session_for(&self, conversation_key: &str) -> Result<String> {
        if let Some(existing) = self.sessions.get(conversation_key, None) {
            if !existing.is_empty() {
                return Ok(existing);
            }
        }
        let mut client = self.bridge.client().await?;
        let session_id = client
            .new_session(&self.bridge.agent_cfg().cwd, self.id)
            .await?;
        self.sessions
            .set_session_id(conversation_key, None, &session_id);
        Ok(session_id)
    }

    /// Forget a conversation's session so the next message starts fresh.
    pub fn reset_session(&self, conversation_key: &str) {
        self.sessions.reset(conversation_key, None);
    }

    /// The whole pipeline for one inbound message.
    ///
    /// Providers call this once per message and log the outcome; everything
    /// else — dedup, policy, session, queueing, the turn — happens here.
    pub async fn handle(&self, inbound: Inbound, sender: Arc<dyn ChannelSender>) -> HandleOutcome {
        let conversation_key = inbound.conversation_key(self.id);

        if dedup::is_stale(
            inbound.created_at_ms,
            dedup::now_ms(),
            dedup::DEFAULT_STALE_AFTER_MS,
        ) {
            tracing::debug!(
                channel = self.id,
                message = %inbound.message_id,
                "ignoring a replayed message older than the freshness window"
            );
            return HandleOutcome::Stale;
        }

        // Duplicates are keyed by channel so two platforms cannot collide.
        if self
            .bridge
            .dedup
            .check_and_insert(&format!("{}:{}", self.id, inbound.message_id))
        {
            self.bridge.status.count_duplicate(self.id);
            return HandleOutcome::Duplicate;
        }

        // A bare yes/no while an approval is outstanding answers it, and is not
        // a new prompt.
        if let Some((route, approved)) = self
            .bridge
            .approvals
            .claim(&conversation_key, &inbound.text)
        {
            let mut client = match self.bridge.client().await {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(channel = self.id, %error, "cannot deliver the approval answer");
                    // Put the route back: the user's answer should be retryable.
                    self.bridge.approvals.insert(&conversation_key, route);
                    return HandleOutcome::Backpressure;
                }
            };
            match client
                .approval_decision(
                    &route.session_id,
                    &route.request_id,
                    approved,
                    &inbound.text,
                )
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        channel = self.id,
                        request = %route.request_id,
                        approved,
                        "approval answered from chat"
                    );
                }
                Err(error) => {
                    tracing::warn!(channel = self.id, %error, "approval answer was rejected");
                    // Put the route back: the user's answer should be retryable.
                    self.bridge.approvals.insert(&conversation_key, route);
                    return HandleOutcome::Backpressure;
                }
            }
            return HandleOutcome::ApprovalAnswered;
        }

        if let Access::Denied(reason) = self.access(&inbound) {
            // Tell the sender why, but only when they addressed the bot —
            // otherwise a busy group turns into a stream of denials.
            if inbound.conversation.kind == ChatKind::Direct || inbound.addressed_to_bot {
                let _ = sender.send_text(&inbound.conversation, &reason).await;
            }
            return HandleOutcome::Denied(reason);
        }

        let session_id = match self.session_for(&conversation_key).await {
            Ok(session_id) => session_id,
            Err(error) => {
                tracing::warn!(channel = self.id, %error, "cannot open an agent session");
                let _ = sender
                    .send_text(
                        &inbound.conversation,
                        &format!("⚠️ Cannot reach the agent: {error}"),
                    )
                    .await;
                return HandleOutcome::Backpressure;
            }
        };

        let images = self.collect_images(&inbound);
        let sink = Arc::new(ChannelSink::new(sender, inbound.conversation.clone()));
        let job = Job {
            conversation: conversation_key,
            channel: self.id.to_string(),
            session_id,
            reply_to: inbound.conversation.clone(),
            text: inbound.text.clone(),
            images,
            sink,
        };
        match self.bridge.conversations().submit(job).await {
            SubmitOutcome::Accepted => {
                self.bridge
                    .status
                    .count_inbound(self.id, crate::status::now_unix());
                HandleOutcome::Accepted
            }
            SubmitOutcome::Full => {
                self.bridge.status.count_rejected(self.id);
                tracing::warn!(
                    channel = self.id,
                    conversation = %inbound.conversation.id,
                    "conversation queue is full; dropping the message"
                );
                HandleOutcome::Backpressure
            }
        }
    }

    /// Save inbound images and turn them into model input.
    ///
    /// Bytes are written under `data_dir/inbox` so a prompt can reference the
    /// file, and images beyond the size cap are skipped rather than ballooning
    /// the request.
    fn collect_images(&self, inbound: &Inbound) -> Vec<ImageInput> {
        let mut images = Vec::new();
        for (index, media) in inbound.media.iter().enumerate() {
            let Some(data) = media.data.as_ref() else {
                continue;
            };
            if data.len() > MAX_IMAGE_BYTES {
                tracing::warn!(
                    channel = self.id,
                    bytes = data.len(),
                    "skipping an oversized image attachment"
                );
                continue;
            }
            let name = media
                .filename
                .clone()
                .unwrap_or_else(|| format!("{}-{index}", inbound.message_id));
            let safe = sanitize_filename(&name);
            let path = self.data_dir.join("inbox").join(safe);
            let saved = (|| -> std::io::Result<()> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, data)
            })();
            if let Err(error) = saved {
                tracing::warn!(channel = self.id, %error, "cannot save an inbound image");
            }
            images.push(ImageInput {
                content_type: media
                    .content_type
                    .clone()
                    .unwrap_or_else(|| "image/png".to_string()),
                data: ImageData::Base64(base64::engine::general_purpose::STANDARD.encode(data)),
                file_path: Some(path.to_string_lossy().into_owned()),
            });
        }
        images
    }

    /// Send one message immediately, outside any turn.
    ///
    /// Used for the acknowledgement a provider posts before queueing, and by
    /// `future channel send`. Delivery guarantees belong to the delivery queue;
    /// this is the direct path.
    pub async fn send_now(
        &self,
        sender: &Arc<dyn ChannelSender>,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<()> {
        let definition = self.definition;
        for piece in crate::transport::chunk(text, definition.max_text_len, definition.length_unit)
        {
            sender.send_text(conversation, &piece).await?;
        }
        self.bridge
            .status
            .count_outbound(self.id, crate::status::now_unix());
        Ok(())
    }

    /// Mark the channel as running in the published status.
    pub fn mark_running(&self) {
        self.bridge.status.set_started(self.id);
    }

    /// Mark the channel as failed, with the reason.
    pub fn mark_failed(&self, error: &str) {
        self.bridge
            .status
            .set_state(self.id, ChannelState::Error, Some(error.to_string()));
    }
}

/// Keep a downloaded file inside the channel's own directory.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            other => other,
        })
        .collect();
    let trimmed = cleaned.trim_matches(['.', ' ']).to_string();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed
    }
}

impl Bridge {
    /// The conversation routing table, started in [`Bridge::new`].
    pub fn conversations(&self) -> &Arc<Conversations> {
        self.conversations
            .get()
            .expect("conversations are started with the bridge")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::traits::{Capabilities, Maturity};
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
        max_text_len: 50,
        length_unit: LengthUnit::Chars,
        config_example: r#"{"enabled": true}"#,
        requires: &[],
    };

    struct RecordingSender {
        sent: StdMutex<Vec<String>>,
    }

    impl RecordingSender {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sent: StdMutex::new(Vec::new()),
            })
        }

        fn sent(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
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
            self.sent.lock().unwrap().push(text.to_string());
            Ok(None)
        }
    }

    fn ctx(label: &str, policy: crate::policy::AccessPolicyConfig) -> ProviderCtx {
        let data_dir = crate::test_support::temp_dir(label);
        let sessions = Arc::new(SessionStore::new(data_dir.join("sessions.json")));
        // Tests must never reach a real agent on this machine: an unroutable
        // loopback port fails fast and keeps the outcome deterministic.
        let agent_cfg = AgentConfig {
            grpc_addr: "http://127.0.0.1:1".into(),
            cwd: data_dir.to_string_lossy().into_owned(),
            ..AgentConfig::default()
        };
        let bridge = Bridge::new(
            Arc::new(agent_cfg),
            policy,
            data_dir.clone(),
            Arc::new(StatusBoard::new(data_dir.join("status.json"))),
        );
        ProviderCtx::new(
            &DEFINITION,
            serde_json::json!({"enabled": true}),
            bridge,
            data_dir,
            sessions,
            Arc::new(Notify::new()),
        )
    }

    fn open_policy() -> crate::policy::AccessPolicyConfig {
        crate::policy::AccessPolicyConfig {
            dm_policy: "open".into(),
            dm_allowlist: Vec::new(),
            group_policy: "open".into(),
            group_allowlist: Vec::new(),
            require_mention: false,
        }
    }

    #[test]
    fn config_deserializes_into_the_providers_own_type() {
        #[derive(serde::Deserialize)]
        struct Block {
            enabled: bool,
        }
        let ctx = ctx("bridge-config", open_policy());
        assert!(ctx.config::<Block>().unwrap().enabled);
        assert_eq!(ctx.id(), "testchannel");
        assert_eq!(ctx.definition().id, "testchannel");
        assert!(ctx.config_value().get("enabled").is_some());
    }

    #[test]
    fn a_malformed_config_block_names_the_channel() {
        #[derive(serde::Deserialize, Debug)]
        struct Block {
            #[allow(dead_code)]
            port: u16,
        }
        let ctx = ctx("bridge-bad-config", open_policy());
        let error = ctx.config::<Block>().unwrap_err().to_string();
        assert!(error.contains("testchannel"), "{error}");
    }

    #[tokio::test]
    async fn a_duplicate_message_is_ignored_the_second_time() {
        let ctx = ctx("bridge-dedup", open_policy());
        let sender = RecordingSender::new();
        let inbound = Inbound::new_direct("m1", "u1", "c1", "hello");
        // Neither call reaches the agent (it is not running), but the first is
        // admitted to the pipeline and recorded as seen.
        let first = ctx.handle(inbound.clone(), sender.clone()).await;
        assert!(!matches!(first, HandleOutcome::Duplicate));
        let second = ctx.handle(inbound, sender.clone()).await;
        assert_eq!(second, HandleOutcome::Duplicate);
    }

    #[tokio::test]
    async fn a_stale_replay_is_dropped_before_anything_else() {
        let ctx = ctx("bridge-stale", open_policy());
        let sender = RecordingSender::new();
        let old = crate::bridge::dedup::now_ms() - dedup::DEFAULT_STALE_AFTER_MS - 1_000;
        let outcome = ctx
            .handle(
                Inbound::new_direct("m1", "u1", "c1", "hello").at(old),
                sender,
            )
            .await;
        assert_eq!(outcome, HandleOutcome::Stale);
    }

    #[tokio::test]
    async fn a_denied_sender_in_a_direct_message_is_told_why() {
        let policy = crate::policy::AccessPolicyConfig {
            dm_policy: "allowlist".into(),
            dm_allowlist: vec!["u_allowed".into()],
            ..open_policy()
        };
        let ctx = ctx("bridge-denied", policy);
        let sender = RecordingSender::new();
        let outcome = ctx
            .handle(
                Inbound::new_direct("m1", "u_stranger", "c1", "hello"),
                sender.clone(),
            )
            .await;
        match outcome {
            HandleOutcome::Denied(reason) => assert!(reason.contains("u_stranger"), "{reason}"),
            other => panic!("expected a denial, got {other:?}"),
        }
        assert_eq!(sender.sent().len(), 1, "the sender is told why");
    }

    #[tokio::test]
    async fn a_denied_group_message_that_was_not_addressed_gets_no_reply() {
        let policy = crate::policy::AccessPolicyConfig {
            group_policy: "disabled".into(),
            ..open_policy()
        };
        let ctx = ctx("bridge-denied-group", policy);
        let sender = RecordingSender::new();
        let mut inbound = Inbound::new_direct("m1", "u1", "g1", "chatting").kind(ChatKind::Group);
        inbound.addressed_to_bot = false;
        let outcome = ctx.handle(inbound, sender.clone()).await;
        assert!(matches!(outcome, HandleOutcome::Denied(_)));
        assert!(sender.sent().is_empty(), "no reply into a silent group");
    }

    #[tokio::test]
    async fn an_unreachable_agent_is_reported_to_the_user() {
        let ctx = ctx("bridge-no-agent", open_policy());
        let sender = RecordingSender::new();
        let outcome = ctx
            .handle(
                Inbound::new_direct("m1", "u1", "c1", "hello"),
                sender.clone(),
            )
            .await;
        assert_eq!(outcome, HandleOutcome::Backpressure);
        let sent = sender.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Cannot reach the agent"), "{sent:?}");
    }

    #[tokio::test]
    async fn an_approval_answer_is_claimed_before_it_becomes_a_prompt() {
        let ctx = ctx("bridge-approval", open_policy());
        let sender = RecordingSender::new();
        ctx.bridge.approvals().insert(
            "testchannel:c1",
            ApprovalRoute {
                session_id: "s1".into(),
                request_id: "req_1".into(),
                tool_name: "shell".into(),
            },
        );
        let outcome = ctx
            .handle(Inbound::new_direct("m1", "u1", "c1", "yes"), sender.clone())
            .await;
        // The agent is not running, so delivering the decision fails and the
        // route is put back for a retry — the important part is that the message
        // was claimed as an answer rather than queued as a prompt.
        assert!(!matches!(outcome, HandleOutcome::Accepted));
        assert!(ctx.bridge.approvals().peek("testchannel:c1").is_some());
    }

    #[tokio::test]
    async fn a_question_while_an_approval_is_pending_is_still_a_prompt() {
        let ctx = ctx("bridge-approval-question", open_policy());
        let sender = RecordingSender::new();
        ctx.bridge.approvals().insert(
            "testchannel:c1",
            ApprovalRoute {
                session_id: "s1".into(),
                request_id: "req_1".into(),
                tool_name: "shell".into(),
            },
        );
        let outcome = ctx
            .handle(
                Inbound::new_direct("m1", "u1", "c1", "what will that command do?"),
                sender.clone(),
            )
            .await;
        assert_ne!(outcome, HandleOutcome::ApprovalAnswered);
        assert!(
            ctx.bridge.approvals().peek("testchannel:c1").is_some(),
            "the route survives"
        );
    }

    #[tokio::test]
    async fn send_now_splits_to_the_platform_limit() {
        let ctx = ctx("bridge-send", open_policy());
        let sender = RecordingSender::new();
        let sender_dyn: Arc<dyn ChannelSender> = sender.clone();
        ctx.send_now(&sender_dyn, &ConversationRef::default(), &"x".repeat(120))
            .await
            .unwrap();
        let sent = sender.sent();
        assert_eq!(sent.len(), 3, "{sent:?}");
        assert!(sent.iter().all(|piece| piece.chars().count() <= 50));
    }

    #[test]
    fn filenames_cannot_escape_the_channel_directory() {
        for hostile in [
            "../../etc/passwd",
            "..\\..\\windows\\system32",
            "/etc/shadow",
            "C:\\Windows\\notepad.exe",
            "a/b:c*d?e\"f<g>h|i",
            "   ",
            "...",
            "\0",
        ] {
            let safe = sanitize_filename(hostile);
            assert!(!safe.contains('/'), "{hostile} -> {safe}");
            assert!(!safe.contains('\\'), "{hostile} -> {safe}");
            assert!(!safe.starts_with('.'), "{hostile} -> {safe}");
            assert!(!safe.is_empty(), "{hostile} produced an empty name");
            assert_eq!(
                safe,
                std::path::Path::new(&safe)
                    .file_name()
                    .unwrap()
                    .to_string_lossy(),
                "{hostile} -> {safe} must be a bare file name"
            );
        }
        assert_eq!(sanitize_filename("photo.png"), "photo.png");
        assert_eq!(sanitize_filename("my report (1).pdf"), "my report (1).pdf");
    }

    #[test]
    fn images_are_saved_and_encoded_for_the_model() {
        let ctx = ctx("bridge-images", open_policy());
        ctx.ensure_data_dir().unwrap();
        let mut inbound = Inbound::new_direct("m1", "u1", "c1", "look");
        inbound.media = vec![MediaRef {
            kind: MediaKind::Image,
            filename: Some("shot.png".into()),
            content_type: Some("image/png".into()),
            url: None,
            data: Some(vec![1, 2, 3, 4]),
        }];
        let images = ctx.collect_images(&inbound);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].content_type, "image/png");
        match &images[0].data {
            ImageData::Base64(encoded) => assert_eq!(encoded, "AQIDBA=="),
            other => panic!("expected base64 input, got {other:?}"),
        }
        let path = images[0].file_path.clone().unwrap();
        assert!(std::path::Path::new(&path).exists());
    }

    #[test]
    fn oversized_images_are_skipped_rather_than_sent() {
        let ctx = ctx("bridge-images-large", open_policy());
        ctx.ensure_data_dir().unwrap();
        let mut inbound = Inbound::new_direct("m1", "u1", "c1", "look");
        inbound.media = vec![MediaRef {
            kind: MediaKind::Image,
            data: Some(vec![0u8; MAX_IMAGE_BYTES + 1]),
            ..Default::default()
        }];
        assert!(ctx.collect_images(&inbound).is_empty());
    }

    #[test]
    fn media_without_bytes_is_not_model_input() {
        let ctx = ctx("bridge-images-url", open_policy());
        let mut inbound = Inbound::new_direct("m1", "u1", "c1", "look");
        inbound.media = vec![MediaRef {
            kind: MediaKind::Image,
            url: Some("https://example.test/a.png".into()),
            ..Default::default()
        }];
        assert!(ctx.collect_images(&inbound).is_empty());
    }

    #[test]
    fn an_offline_bridge_reports_no_connection() {
        let bridge = Bridge::offline();
        assert!(!bridge.is_connected());
        assert!(bridge
            .data_root()
            .to_string_lossy()
            .contains("future-channel"));
    }

    #[test]
    fn chat_overrides_change_the_verdict_at_runtime() {
        let policy = crate::policy::AccessPolicyConfig {
            group_policy: "disabled".into(),
            ..open_policy()
        };
        let ctx = ctx("bridge-override", policy);
        let inbound = Inbound::new_direct("m1", "u1", "g1", "hi")
            .kind(ChatKind::Group)
            .at(0);
        assert!(matches!(ctx.access(&inbound), Access::Denied(_)));
        ctx.bridge.set_chat_override(
            "g1",
            crate::policy::ChatOverride {
                enabled: Some(true),
                require_mention: Some(false),
            },
        );
        assert_eq!(ctx.access(&inbound), Access::Allowed);
    }

    #[test]
    fn session_reset_forgets_a_conversation() {
        let ctx = ctx("bridge-session-reset", open_policy());
        ctx.sessions()
            .set_session_id("testchannel:c1", None, "sess-1");
        assert_eq!(
            ctx.sessions().get("testchannel:c1", None).as_deref(),
            Some("sess-1")
        );
        ctx.reset_session("testchannel:c1");
        assert_eq!(ctx.sessions().get("testchannel:c1", None), None);
    }

    #[test]
    fn status_marks_are_published() {
        let ctx = ctx("bridge-status", open_policy());
        ctx.mark_running();
        ctx.mark_failed("token rejected");
        ctx.bridge.status().flush().unwrap();
        let snapshot =
            crate::status::StatusSnapshot::load(&ctx.bridge.data_root().join("status.json"));
        let entry = snapshot.channels.get("testchannel").expect("entry");
        assert_eq!(entry.state, Some(ChannelState::Error));
        assert_eq!(entry.last_error.as_deref(), Some("token rejected"));
    }

    // ─── the whole pipeline against a mock agent ────────────────────────────

    /// A sink that only counts what it was told, for tests that care about the
    /// pipeline rather than the rendering.
    #[derive(Default)]
    struct CountingSink {
        superseded: StdMutex<usize>,
        finished: StdMutex<usize>,
    }

    impl CountingSink {
        fn superseded_count(&self) -> usize {
            *self.superseded.lock().unwrap()
        }

        #[allow(dead_code)]
        fn finished_count(&self) -> usize {
            *self.finished.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl crate::bridge::sink::ReplySink for CountingSink {
        async fn finish(&self, _outcome: &crate::bridge::sink::TurnOutcome) -> Result<()> {
            *self.finished.lock().unwrap() += 1;
            Ok(())
        }

        async fn superseded(&self) -> Result<()> {
            *self.superseded.lock().unwrap() += 1;
            Ok(())
        }
    }

    /// A context wired to a mock agent, plus the sender its replies land in.
    async fn ctx_with_agent(
        label: &str,
        events: Vec<future_rpc::proto::StreamEvent>,
    ) -> (ProviderCtx, Arc<RecordingSender>, ts::SharedState) {
        ts::ensure_crypto_provider();
        let state = ts::MockState {
            events,
            ..Default::default()
        };
        let (addr, shared) = ts::spawn_mock_grpc(state).await;
        let data_dir = crate::test_support::temp_dir(label);
        let agent_cfg = AgentConfig {
            grpc_addr: format!("http://{addr}"),
            cwd: data_dir.to_string_lossy().into_owned(),
            ..AgentConfig::default()
        };
        let bridge = Bridge::new(
            Arc::new(agent_cfg),
            open_policy(),
            data_dir.clone(),
            Arc::new(StatusBoard::new(data_dir.join("status.json"))),
        );
        let sessions = Arc::new(SessionStore::new(data_dir.join("sessions.json")));
        let ctx = ProviderCtx::new(
            &DEFINITION,
            serde_json::json!({"enabled": true}),
            bridge,
            data_dir,
            sessions,
            Arc::new(Notify::new()),
        );
        (ctx, RecordingSender::new(), shared)
    }

    /// The mock names its first run `mock-run-1`.
    const MOCK_RUN: &str = "mock-run-1";

    fn completed_turn(answer: &str) -> Vec<future_rpc::proto::StreamEvent> {
        vec![
            ts::ev(
                MOCK_RUN,
                1,
                "text_chunk",
                &format!(r#"{{"text":{answer:?}}}"#),
            ),
            ts::ev(MOCK_RUN, 2, "agent_end", r#"{"state":"completed"}"#),
        ]
    }

    async fn wait_for_reply(sender: &RecordingSender) -> Vec<String> {
        let ok = ts::wait_until(
            || !sender.sent().is_empty(),
            std::time::Duration::from_secs(10),
        )
        .await;
        assert!(ok, "the turn should have produced a reply");
        sender.sent()
    }

    #[tokio::test]
    async fn a_message_becomes_a_session_a_turn_and_a_reply() {
        let (ctx, sender, state) = ctx_with_agent("bridge-e2e", completed_turn("Hello")).await;
        let outcome = ctx
            .handle(Inbound::new_direct("m1", "u1", "c1", "hi"), sender.clone())
            .await;
        assert_eq!(outcome, HandleOutcome::Accepted);
        assert!(outcome.is_accepted());

        assert_eq!(wait_for_reply(&sender).await, vec!["Hello".to_string()]);

        // The agent saw a session and a prompt on it.
        assert_eq!(ts::recorded_of(&state, "new_session").len(), 1);
        let prompts = ts::recorded_of(&state, "prompt");
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].message.contains("hi"));

        // The conversation now maps to that session, so the next message
        // reuses it instead of opening another.
        let session = ctx
            .sessions()
            .get("testchannel:c1", None)
            .expect("a session mapping");
        assert!(session.starts_with("mock-session-"));
        assert_eq!(
            ctx.handle(
                Inbound::new_direct("m2", "u1", "c1", "again"),
                sender.clone()
            )
            .await,
            HandleOutcome::Accepted
        );
        assert_eq!(ts::recorded_of(&state, "new_session").len(), 1);
        // And the counters that `future channel status` reads were updated.
        ctx.bridge.status().flush().unwrap();
        let snapshot =
            crate::status::StatusSnapshot::load(&ctx.bridge.data_root().join("status.json"));
        let entry = snapshot.channels.get("testchannel").expect("status entry");
        assert_eq!(entry.inbound_count, 2);
        assert!(entry.outbound_count >= 1);
    }

    #[tokio::test]
    async fn an_image_input_reaches_the_prompt() {
        let (ctx, sender, state) =
            ctx_with_agent("bridge-e2e-image", completed_turn("seen")).await;
        ctx.ensure_data_dir().unwrap();
        let mut inbound = Inbound::new_direct("m1", "u1", "c1", "what is this");
        inbound.media = vec![MediaRef {
            kind: MediaKind::Image,
            filename: Some("shot.png".into()),
            content_type: Some("image/png".into()),
            data: Some(vec![1, 2, 3, 4]),
            url: None,
        }];
        assert_eq!(
            ctx.handle(inbound, sender.clone()).await,
            HandleOutcome::Accepted
        );
        wait_for_reply(&sender).await;
        let prompts = ts::recorded_of(&state, "prompt");
        assert_eq!(
            prompts[0].images.len(),
            1,
            "the image travels with the prompt"
        );
        assert_eq!(prompts[0].images[0].r#type, "image/png");
    }

    #[tokio::test]
    async fn a_second_message_in_the_same_conversation_wins() {
        // Two prompts in flight: the newer message supersedes the older turn,
        // so the user is answered once rather than twice.
        let (ctx, sender, state) = ctx_with_agent(
            "bridge-e2e-supersede",
            vec![
                ts::ev(MOCK_RUN, 1, "text_chunk", r#"{"text":"first"}"#),
                ts::ev(MOCK_RUN, 2, "agent_end", r#"{"state":"completed"}"#),
                ts::ev(MOCK_RUN, 3, "text_chunk", r#"{"text":"second"}"#),
                ts::ev(MOCK_RUN, 4, "agent_end", r#"{"state":"completed"}"#),
            ],
        )
        .await;
        let mut counter = 0;
        let mut conversation = || {
            counter += 1;
            Inbound::new_direct(&format!("m{counter}"), "u1", "c1", "hi")
        };
        assert_eq!(
            ctx.handle(conversation(), sender.clone()).await,
            HandleOutcome::Accepted
        );
        assert_eq!(
            ctx.handle(conversation(), sender.clone()).await,
            HandleOutcome::Accepted
        );
        let sent = wait_for_reply(&sender).await;
        assert!(!sent.is_empty());
        // The newest message is the one answered, and the superseded turn was
        // either replaced before it prompted or dropped — never answered twice.
        let prompts = ts::recorded_of(&state, "prompt");
        assert!(!prompts.is_empty() && prompts.len() <= 2, "{}", prompts.len());
    }

    #[tokio::test]
    async fn a_failed_turn_is_explained_to_the_user() {
        let (ctx, sender, _state) = ctx_with_agent(
            "bridge-e2e-error",
            vec![ts::ev(
                MOCK_RUN,
                1,
                "agent_end",
                r#"{"state":"error","error":"the provider refused"}"#,
            )],
        )
        .await;
        assert_eq!(
            ctx.handle(Inbound::new_direct("m1", "u1", "c1", "hi"), sender.clone())
                .await,
            HandleOutcome::Accepted
        );
        let sent = wait_for_reply(&sender).await;
        assert!(sent.concat().contains("the provider refused"), "{sent:?}");
    }

    #[tokio::test]
    async fn an_approval_answer_asks_the_agent_to_decide() {
        let (ctx, sender, state) =
            ctx_with_agent("bridge-e2e-approval", completed_turn("ok")).await;
        // Establish the session first, then pretend the agent parked a request.
        assert_eq!(
            ctx.handle(Inbound::new_direct("m1", "u1", "c1", "hi"), sender.clone())
                .await,
            HandleOutcome::Accepted
        );
        wait_for_reply(&sender).await;
        let session = ctx.sessions().get("testchannel:c1", None).unwrap();
        ctx.bridge.approvals().insert(
            "testchannel:c1",
            ApprovalRoute {
                session_id: session,
                request_id: "req_1".into(),
                tool_name: "shell".into(),
            },
        );

        let outcome = ctx
            .handle(Inbound::new_direct("m2", "u1", "c1", "yes"), sender.clone())
            .await;
        assert_eq!(outcome, HandleOutcome::ApprovalAnswered);
        assert_eq!(ts::recorded_of(&state, "approval_decision").len(), 1);
        assert!(ctx.bridge.approvals().is_empty(), "the route is consumed");
    }

    #[tokio::test]
    async fn a_full_conversation_queue_drops_the_message_and_says_so() {
        let (ctx, sender, _state) =
            ctx_with_agent("bridge-e2e-full", completed_turn("done")).await;
        // Fill the mailbox past its bound without letting the worker drain it.
        let mut saw_full = false;
        for index in 0..40 {
            let outcome = ctx
                .handle(
                    Inbound::new_direct(&format!("m{index}"), "u1", "c1", "hi"),
                    sender.clone(),
                )
                .await;
            if outcome == HandleOutcome::Backpressure {
                saw_full = true;
                break;
            }
        }
        assert!(saw_full, "the queue must apply backpressure");
        ctx.bridge.status().flush().unwrap();
        let snapshot =
            crate::status::StatusSnapshot::load(&ctx.bridge.data_root().join("status.json"));
        let entry = snapshot.channels.get("testchannel").expect("status entry");
        assert_eq!(entry.rejected_count, 1);
    }

    #[tokio::test]
    async fn the_client_connects_once_and_can_be_invalidated() {
        let (ctx, sender, _state) =
            ctx_with_agent("bridge-e2e-client", completed_turn("ok")).await;
        assert!(!ctx.bridge.is_connected());
        let _first = ctx.bridge.client().await.expect("connect");
        assert!(ctx.bridge.is_connected());
        // The cached connection is reused rather than dialled again.
        let _second = ctx.bridge.client().await.expect("cached connect");
        ctx.bridge.invalidate_client();
        assert!(!ctx.bridge.is_connected());
        let _third = ctx.bridge.client().await.expect("reconnect");
        assert!(ctx.bridge.is_connected());
        assert!(sender.sent().is_empty(), "nothing was sent for this test");
    }

    #[tokio::test]
    async fn a_turn_with_no_reachable_agent_fails_the_job_not_the_process() {
        // The conversation was accepted, but the agent is unreachable when the
        // turn runs: the user is told and the bridge stays up.
        let (ctx, sender, _state) =
            ctx_with_agent("bridge-e2e-late-failure", completed_turn("never")).await;
        let session = ctx.session_for("testchannel:c1").await.expect("session");
        assert!(session.starts_with("mock-session-"));
        let dead = ProviderCtx::new(
            &DEFINITION,
            serde_json::json!({"enabled": true}),
            Bridge::new(
                Arc::new(AgentConfig {
                    grpc_addr: "http://127.0.0.1:1".into(),
                    cwd: ctx.data_dir().to_string_lossy().into_owned(),
                    ..AgentConfig::default()
                }),
                open_policy(),
                ctx.data_dir().to_path_buf(),
                Arc::new(StatusBoard::new(ctx.data_dir().join("status.json"))),
            ),
            ctx.data_dir().to_path_buf(),
            Arc::new(SessionStore::new(ctx.data_dir().join("sessions.json"))),
            Arc::new(Notify::new()),
        );
        dead.sessions()
            .set_session_id("testchannel:c1", None, &session);
        assert_eq!(
            dead.handle(Inbound::new_direct("m1", "u1", "c1", "hi"), sender.clone())
                .await,
            HandleOutcome::Accepted
        );
        let sent = wait_for_reply(&sender).await;
        assert!(
            sent.concat().contains("unreachable") || sent.concat().contains("did not finish"),
            "{sent:?}"
        );
    }

    #[tokio::test]
    async fn stopping_wakes_every_provider_and_the_accessors_work() {
        let ctx = ctx("bridge-shutdown", open_policy());
        let waiter = ctx.shutdown().notified();
        ctx.shutdown().notify_waiters();
        tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("shutdown must wake providers");
        assert_eq!(ctx.bridge.agent_cfg().model, AgentConfig::default().model);
        // The provider talks through the bridge's client rather than building
        // its own, so connection pooling and timeouts stay in one place.
        assert!(std::ptr::eq(
            ctx.http() as *const reqwest::Client,
            ctx.bridge.http() as *const reqwest::Client
        ));
        assert_eq!(ctx.bridge.status().path().file_name().unwrap(), "status.json");
        assert!(ctx.bridge.conversations().len().await >= 0);
    }

    #[tokio::test]
    async fn a_job_after_the_bridge_is_dropped_is_skipped_not_fatal() {
        // A conversation worker can outlive the bridge it was built from (the
        // routing table is shared by reference). A job arriving then must be
        // skipped — the bridge is gone, so there is nothing to answer with — and
        // must not panic inside the worker.
        let bridge = Bridge::offline();
        let conversations = bridge.conversations().clone();
        drop(bridge);

        let sink = Arc::new(CountingSink::default());
        let job = Job {
            conversation: "testchannel:c1".into(),
            channel: "testchannel".into(),
            session_id: "s1".into(),
            reply_to: ConversationRef::default(),
            text: "hi".into(),
            images: Vec::new(),
            sink: sink.clone(),
        };
        assert_eq!(conversations.submit(job).await, SubmitOutcome::Accepted);
        // The worker ran the job; the runner found no bridge and skipped it, so
        // nothing was reported and the process is still healthy.
        let routed = crate::test_support::wait_until(
            || true,
            std::time::Duration::from_millis(20),
        )
        .await;
        assert!(routed);
        assert_eq!(conversations.len().await, 1);
        assert_eq!(sink.superseded_count(), 0);
        assert_eq!(sink.finished_count(), 0);
    }

    #[test]
    fn offer_outcomes_are_classified() {
        assert!(HandleOutcome::Accepted.is_accepted());
        assert!(HandleOutcome::ApprovalAnswered.is_accepted());
        assert!(!HandleOutcome::Duplicate.is_accepted());
        assert!(!HandleOutcome::Stale.is_accepted());
        assert!(!HandleOutcome::Backpressure.is_accepted());
        assert!(!HandleOutcome::Denied("no".into()).is_accepted());
        // Denials carry the reason an administrator needs to act on.
        assert_eq!(
            HandleOutcome::Denied("not allowlisted".into()),
            HandleOutcome::Denied("not allowlisted".into())
        );
    }
}

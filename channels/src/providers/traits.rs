//! What a channel provider implements.
//!
//! A provider is two halves plus metadata:
//!
//! * **Inbound** — whatever the platform offers (a websocket, a long poll, a
//!   webhook) turns into [`crate::bridge::Inbound`], and the provider hands it
//!   to [`crate::bridge::ProviderCtx::handle`]. Everything after that —
//!   duplicate filtering, access policy, session routing, queueing, streaming
//!   the answer back — is the bridge's job, not the provider's.
//! * **Outbound** — a [`ChannelSender`]: send text, optionally edit it in
//!   place, optionally type or react. Chunking, throttling and progressive
//!   editing of a streamed answer are the bridge's job; the sender only has to
//!   talk to the platform.
//!
//! Keeping that line is what makes a new IM cheap: the provider file holds
//! platform knowledge, and nothing else.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::bridge::{ConversationRef, ProviderCtx};
use crate::transport::LengthUnit;

/// How complete a provider is. Surfaced by `future channel list` so a preview
/// channel is never mistaken for a verified one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maturity {
    /// Recognized but not implemented in this build; enabling it is an error.
    Planned,
    /// Implemented against the platform's public API, not yet exercised against
    /// a live deployment.
    Preview,
    /// Exercised against a live deployment; safe to rely on.
    Live,
}

impl Maturity {
    pub fn as_str(self) -> &'static str {
        match self {
            Maturity::Planned => "planned",
            Maturity::Preview => "preview",
            Maturity::Live => "live",
        }
    }

    pub fn is_usable(self) -> bool {
        !matches!(self, Maturity::Planned)
    }
}

/// What a channel can do, in bridge terms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Capabilities {
    /// Accept messages from the platform.
    pub receive: bool,
    /// Send messages to the platform.
    pub send: bool,
    /// Rewrite an already-sent message, enabling progressive streaming.
    pub edit: bool,
    /// Reply inside a thread rather than at the top of the conversation.
    pub threads: bool,
    /// Show a typing/progress signal before the reply exists.
    pub typing: bool,
    /// Add and remove emoji reactions (used as a lightweight acknowledgement).
    pub reactions: bool,
    /// Accept attachments from the platform.
    pub media_in: bool,
    /// Send attachments to the platform.
    pub media_out: bool,
    /// Reachable only by being addressed (group mention gate applies).
    pub mention_gate: bool,
}

impl Capabilities {
    /// Receive and send text, nothing else — the floor for a usable channel.
    pub const TEXT: Self = Self {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: false,
        media_in: false,
        media_out: false,
        mention_gate: true,
    };

    /// The usual modern chat client: text, editing, threads, typing.
    pub const RICH: Self = Self {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    };
}

/// One channel's declaration: identity, limits, and what it can do.
#[derive(Debug, Clone, Copy)]
pub struct ChannelDefinition {
    /// Stable machine id; also the key of its `providers` config block.
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    /// Where to read about it: a path under `docs/`, optionally with the anchor
    /// of the section that describes this channel. `every_channel_documents_
    /// itself_somewhere_that_exists` fails the build if the target is not there,
    /// because this value is user-facing (`future channel list --json`).
    pub docs: &'static str,
    pub maturity: Maturity,
    pub capabilities: Capabilities,
    /// Longest single message the platform accepts, in [`Self::length_unit`].
    pub max_text_len: usize,
    pub length_unit: LengthUnit,
    /// A minimal `providers.<id>` block; the per-channel docs and tests mirror it.
    pub config_example: &'static str,
    /// External software or account requirements (empty for a plain HTTP API).
    pub requires: &'static [&'static str],
}

impl ChannelDefinition {
    /// Whether this build can start the channel.
    pub fn is_implemented(&self) -> bool {
        self.maturity.is_usable()
    }

    /// The failure to report when something tries to use this channel.
    ///
    /// One place decides this so the starter, the diagnostics and the outbound
    /// queue cannot drift apart in what they say — and so the wording is
    /// testable without a channel that happens to be unimplemented.
    pub fn ensure_usable(&self) -> Result<(), String> {
        if self.is_implemented() {
            return Ok(());
        }
        Err(format!(
            "the {} channel is not implemented in this build",
            self.id
        ))
    }
}

/// The outbound half of a channel.
///
/// Implementations talk to the platform and nothing more: no chunking (the
/// bridge splits against [`ChannelDefinition::max_text_len`]), no throttling, no
/// retry policy (the bridge and the delivery queue own those).
#[async_trait]
pub trait ChannelSender: Send + Sync {
    /// This sender's channel declaration — the bridge reads limits from here.
    fn definition(&self) -> &'static ChannelDefinition;

    /// Deliver one message. Returning the platform message id lets the bridge
    /// edit it later; platforms that give none return `None` and get no
    /// progressive updates.
    async fn send_text(&self, conversation: &ConversationRef, text: &str)
        -> Result<Option<String>>;

    /// Replace the text of a message this sender previously created.
    ///
    /// The default refuses: a channel that does not advertise
    /// [`Capabilities::edit`] must not silently pretend an update happened.
    async fn edit_text(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let _ = (conversation, message_id, text);
        anyhow::bail!(
            "editing messages is not supported by the {} channel",
            self.definition().id
        )
    }

    /// Show a progress signal. Best-effort: a failure is logged, not fatal.
    async fn typing(&self, conversation: &ConversationRef) -> Result<()> {
        let _ = conversation;
        Ok(())
    }

    /// Add an acknowledgement reaction. Best-effort, like [`Self::typing`].
    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let _ = (conversation, message_id, emoji);
        Ok(())
    }
}

/// A channel implementation.
#[async_trait]
pub trait Provider: Send + Sync {
    /// The declarative half: identity, capabilities, limits.
    fn definition(&self) -> &'static ChannelDefinition;

    /// Build the outbound half from the provider's config.
    ///
    /// Used by `run`, by `future channel test` (connectivity probe) and by
    /// `future channel send` (proactive delivery), so construction must not
    /// start any long-running work.
    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>>;

    /// Start receiving. Returns when the provider stops or fails fatally; the
    /// supervisor reconnects on failure.
    async fn run(&self, ctx: ProviderCtx) -> Result<()>;

    /// Credential/connectivity self-check for `future channel test`.
    ///
    /// On success return a one-line human summary (`"connected as @bot"`). The
    /// default reports that no probe exists rather than claiming success.
    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let _ = ctx;
        anyhow::bail!(
            "the {} channel has no connectivity probe",
            self.definition().id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_planned_channels_are_unusable() {
        assert!(!Maturity::Planned.is_usable());
        assert!(Maturity::Preview.is_usable());
        assert!(Maturity::Live.is_usable());
        assert_eq!(Maturity::Live.as_str(), "live");
    }

    #[test]
    fn planned_maturity_is_rejected_by_the_definition() {
        let planned = ChannelDefinition {
            id: "x",
            display_name: "X",
            description: "",
            docs: "",
            maturity: Maturity::Planned,
            capabilities: Capabilities::TEXT,
            max_text_len: 100,
            length_unit: LengthUnit::Chars,
            config_example: "{}",
            requires: &[],
        };
        assert!(!planned.is_implemented());
        // The refusal names the channel, so an operator can act on it.
        let reason = planned.ensure_usable().expect_err("must refuse");
        assert!(reason.contains('x'), "{reason}");
        assert!(reason.contains("not implemented"), "{reason}");

        let preview = ChannelDefinition {
            maturity: Maturity::Preview,
            ..planned
        };
        assert!(preview.is_implemented());
        assert!(preview.ensure_usable().is_ok());
    }

    #[test]
    fn maturity_names_are_stable_and_usable() {
        assert_eq!(Maturity::Planned.as_str(), "planned");
        assert_eq!(Maturity::Preview.as_str(), "preview");
        assert_eq!(Maturity::Live.as_str(), "live");
        assert!(!Maturity::Planned.is_usable());
        assert!(Maturity::Preview.is_usable());
        assert!(Maturity::Live.is_usable());
    }

    #[test]
    fn the_default_probe_refuses_rather_than_claiming_success() {
        // A provider without a real connectivity check must not report ok: a
        // false "healthy" is worse than an honest "no probe exists".
        struct NoProbe;

        #[async_trait]
        impl Provider for NoProbe {
            fn definition(&self) -> &'static ChannelDefinition {
                &MINIMAL
            }

            fn sender(&self, _ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
                anyhow::bail!("not used")
            }

            async fn run(&self, _ctx: ProviderCtx) -> Result<()> {
                anyhow::bail!("not used")
            }
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ctx = crate::bridge::ProviderCtx::offline(&MINIMAL);
        let error = runtime
            .block_on(async { NoProbe.probe(&ctx).await })
            .expect_err("the default probe must fail")
            .to_string();
        assert!(error.contains("no connectivity probe"), "{error}");
        assert!(error.contains("minimal"), "{error}");
        // The other two entry points are the test double's own, and must fail
        // rather than look functional.
        assert!(NoProbe.sender(&ctx).is_err());
        let run = runtime.block_on(NoProbe.run(ctx));
        assert!(run.is_err());
    }

    #[tokio::test]
    async fn a_minimal_sender_delivers_text_without_a_platform_id() {
        // A platform that returns no message id must still be usable; the
        // bridge then simply has nothing to edit later.
        let sender = MinimalSender {
            definition: &MINIMAL,
        };
        let conversation = ConversationRef::default();
        assert_eq!(sender.definition().max_text_len, 10);
        assert_eq!(sender.send_text(&conversation, "hi").await.unwrap(), None);
    }

    #[test]
    fn the_preset_capability_sets_round_trip_through_their_wire_shape() {
        let text = serde_json::to_value(Capabilities::TEXT).unwrap();
        assert_eq!(text["receive"], serde_json::json!(true));
        assert_eq!(text["send"], serde_json::json!(true));
        assert_eq!(text["edit"], serde_json::json!(false));
        assert_eq!(text["mediaIn"], serde_json::json!(false));
        assert_eq!(text["mentionGate"], serde_json::json!(true));

        let rich = serde_json::to_value(Capabilities::RICH).unwrap();
        assert_eq!(rich["edit"], serde_json::json!(true));
        assert_eq!(rich["threads"], serde_json::json!(true));
        assert_eq!(rich["reactions"], serde_json::json!(true));
        assert_eq!(rich["mediaIn"], serde_json::json!(true));
    }

    struct MinimalSender {
        definition: &'static ChannelDefinition,
    }

    static MINIMAL: ChannelDefinition = ChannelDefinition {
        id: "minimal",
        display_name: "Minimal",
        description: "test double",
        docs: "",
        maturity: Maturity::Preview,
        capabilities: Capabilities::TEXT,
        max_text_len: 10,
        length_unit: LengthUnit::Chars,
        config_example: "{}",
        requires: &[],
    };

    #[async_trait]
    impl ChannelSender for MinimalSender {
        fn definition(&self) -> &'static ChannelDefinition {
            self.definition
        }

        async fn send_text(
            &self,
            _conversation: &ConversationRef,
            _text: &str,
        ) -> Result<Option<String>> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn edit_typing_and_reaction_default_to_harmless_or_explicit() {
        let sender = MinimalSender {
            definition: &MINIMAL,
        };
        let conversation = ConversationRef::default();
        // The edit default is an error: a channel that cannot edit must say so.
        let error = sender
            .edit_text(&conversation, "m1", "text")
            .await
            .expect_err("edit must not silently succeed");
        assert!(error.to_string().contains("not supported"), "{error}");
        // Typing and reactions are best-effort no-ops.
        assert!(sender.typing(&conversation).await.is_ok());
        assert!(sender.react(&conversation, "m1", "👀").await.is_ok());
    }
}

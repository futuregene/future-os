//! The normalized inbound message.
//!
//! Every platform delivers something different: a Feishu event, a Telegram
//! update, a Slack envelope, an SMTP message. The bridge only needs one shape,
//! and providers are the only place platform JSON is understood. Everything
//! after this point — dedup, policy, session routing, the turn — is written
//! once against [`Inbound`].
//!
//! The raw payload is kept alongside the normalized fields so a provider can
//! reach platform-specific extras (an interactive callback, an attachment id)
//! without widening the common type.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What kind of conversation the message arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    /// One-to-one conversation.
    #[default]
    Direct,
    /// Multi-party conversation where the bot must be addressed.
    Group,
    /// Broadcast-style channel (Slack channel, Discord channel, Telegram channel).
    Channel,
}

impl ChatKind {
    /// Whether the mention gate applies.
    pub fn needs_mention(self) -> bool {
        !matches!(self, ChatKind::Direct)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ChatKind::Direct => "direct",
            ChatKind::Group => "group",
            ChatKind::Channel => "channel",
        }
    }
}

/// Who wrote the message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderRef {
    /// Platform-stable identifier (the only field policy can rely on).
    pub id: String,
    /// Human-readable name, when the platform offers one.
    pub display: Option<String>,
}

/// Where the message lives, and where a reply goes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRef {
    /// Platform conversation id (chat id, channel id, mailbox).
    pub id: String,
    /// Thread within the conversation, when the platform threads replies.
    pub thread_id: Option<String>,
    pub kind: ChatKind,
}

impl ConversationRef {
    /// Stable key for session mapping, queueing, and approval routing.
    ///
    /// A thread is its own conversation: two threads of one channel get
    /// separate agent sessions, which is what a user expects in Slack or
    /// Discord.
    pub fn key(&self, channel: &str) -> String {
        match self.thread_id.as_deref().filter(|id| !id.is_empty()) {
            Some(thread) => format!("{channel}:{}:{thread}", self.id),
            None => format!("{channel}:{}", self.id),
        }
    }
}

/// Provider-neutral media kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    Document,
    #[default]
    Unknown,
}

/// One attachment on an inbound message.
///
/// A provider either downloads the bytes (it holds the platform credentials) or
/// records a URL; the bridge turns image bytes into model input and otherwise
/// leaves the reference for the prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaRef {
    pub kind: MediaKind,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub url: Option<String>,
    /// Fetched bytes, when the provider could download without a second round
    /// trip the user would notice.
    pub data: Option<Vec<u8>>,
}

/// One normalized inbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inbound {
    pub message_id: String,
    pub sender: SenderRef,
    pub conversation: ConversationRef,
    /// Message text with platform markup already reduced to plain text.
    pub text: String,
    #[serde(default)]
    pub media: Vec<MediaRef>,
    /// Whether the bot was explicitly addressed (`@bot`, a reply to it, a DM).
    pub addressed_to_bot: bool,
    /// Platform creation time in Unix milliseconds, when available.
    pub created_at_ms: Option<i64>,
    /// The untouched platform payload.
    #[serde(default)]
    pub raw: Option<Value>,
}

impl Inbound {
    /// A text-only direct message — the shape most tests need.
    pub fn new_direct(message_id: &str, sender: &str, conversation: &str, text: &str) -> Self {
        Self {
            message_id: message_id.to_string(),
            sender: SenderRef {
                id: sender.to_string(),
                display: None,
            },
            conversation: ConversationRef {
                id: conversation.to_string(),
                thread_id: None,
                kind: ChatKind::Direct,
            },
            text: text.to_string(),
            media: Vec::new(),
            addressed_to_bot: true,
            created_at_ms: None,
            raw: None,
        }
    }

    /// Attach a thread.
    pub fn in_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.conversation.thread_id = Some(thread_id.into());
        self
    }

    /// Set the conversation kind.
    pub fn kind(mut self, kind: ChatKind) -> Self {
        self.conversation.kind = kind;
        self
    }

    /// Set the sender's display name.
    pub fn named(mut self, display: impl Into<String>) -> Self {
        self.sender.display = Some(display.into());
        self
    }

    /// Set the platform timestamp (Unix milliseconds).
    pub fn at(mut self, created_at_ms: i64) -> Self {
        self.created_at_ms = Some(created_at_ms);
        self
    }

    /// The conversation key this message routes to.
    pub fn conversation_key(&self, channel: &str) -> String {
        self.conversation.key(channel)
    }

    /// Image attachments the model can accept.
    pub fn images(&self) -> Vec<MediaRef> {
        self.media
            .iter()
            .filter(|media| media.kind == MediaKind::Image)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_is_its_own_conversation() {
        let plain = ConversationRef {
            id: "C1".into(),
            thread_id: None,
            kind: ChatKind::Channel,
        };
        let threaded = ConversationRef {
            id: "C1".into(),
            thread_id: Some("T9".into()),
            kind: ChatKind::Channel,
        };
        assert_eq!(plain.key("slack"), "slack:C1");
        assert_eq!(threaded.key("slack"), "slack:C1:T9");
        assert_ne!(plain.key("slack"), threaded.key("slack"));
    }

    #[test]
    fn an_empty_thread_id_is_not_a_thread() {
        let conversation = ConversationRef {
            id: "C1".into(),
            thread_id: Some(String::new()),
            kind: ChatKind::Direct,
        };
        assert_eq!(conversation.key("slack"), "slack:C1");
    }

    #[test]
    fn keys_are_namespaced_per_channel() {
        let inbound = Inbound::new_direct("m1", "u1", "same-id", "hi");
        assert_ne!(
            inbound.conversation_key("telegram"),
            inbound.conversation_key("discord")
        );
    }

    #[test]
    fn mention_gate_applies_outside_direct_messages() {
        assert!(!ChatKind::Direct.needs_mention());
        assert!(ChatKind::Group.needs_mention());
        assert!(ChatKind::Channel.needs_mention());
        assert_eq!(ChatKind::Group.as_str(), "group");
    }

    #[test]
    fn chat_kind_names_are_stable() {
        assert_eq!(ChatKind::Direct.as_str(), "direct");
        assert_eq!(ChatKind::Group.as_str(), "group");
        assert_eq!(ChatKind::Channel.as_str(), "channel");
    }

    #[test]
    fn builders_set_the_expected_fields() {
        let inbound = Inbound::new_direct("m1", "u1", "c1", "hello")
            .in_thread("t1")
            .kind(ChatKind::Group)
            .named("Alice")
            .at(1700);
        assert_eq!(inbound.message_id, "m1");
        assert_eq!(inbound.sender.display.as_deref(), Some("Alice"));
        assert_eq!(inbound.conversation.thread_id.as_deref(), Some("t1"));
        assert_eq!(inbound.conversation.kind, ChatKind::Group);
        assert_eq!(inbound.created_at_ms, Some(1700));
        assert_eq!(inbound.conversation_key("x"), "x:c1:t1");
    }

    #[test]
    fn images_are_selected_by_kind() {
        let mut inbound = Inbound::new_direct("m1", "u1", "c1", "look");
        inbound.media = vec![
            MediaRef {
                kind: MediaKind::Image,
                ..Default::default()
            },
            MediaRef {
                kind: MediaKind::Document,
                ..Default::default()
            },
        ];
        assert_eq!(inbound.images().len(), 1);
    }

    #[test]
    fn the_default_chat_kind_is_direct() {
        assert_eq!(ChatKind::default(), ChatKind::Direct);
        assert_eq!(MediaKind::default(), MediaKind::Unknown);
    }
}

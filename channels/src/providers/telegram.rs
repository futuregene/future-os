//! Telegram: Bot API over long polling or a webhook.
//!
//! Plan of record for the implementation:
//!
//! * **Inbound** — `getUpdates` long polling with a persisted offset (so a
//!   restart does not lose messages the platform already queued), or a webhook
//!   served by [`crate::transport::webhook`] when the deployment can expose one.
//! * **Outbound** — `sendMessage` split to Telegram's 4096-character limit,
//!   MarkdownV2 escaping, `editMessageText` for progressive streaming, and
//!   `sendChatAction` for the typing signal.
//! * **Addressing** — a group message counts as addressed when the bot is
//!   mentioned (via `entities`) or when it replies to one of our messages.
//! * **Media** — `getFile` into the channel's data directory; images become
//!   model input.
//! * **Errors** — 429 carries `retry_after` and must be honoured; 403
//!   (`bot was blocked`) is permanent.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "telegram",
    display_name: "Telegram",
    description: "Telegram bot: long polling or webhook, streamed replies, group mention gate.",
    docs: "docs/guide/channels-telegram.md",
    maturity: Maturity::Planned,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: false,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4096,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "bot_token": "",
  "mode": "long_poll",
  "webhook": { "addr": "127.0.0.1:8787", "path": "/telegram", "secret_token": "" },
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true,
  "streaming": true
}"#,
    requires: &["a bot token from @BotFather"],
};

planned_provider!(DEFINITION);

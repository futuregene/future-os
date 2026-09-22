//! Discord: REST v10 plus the gateway websocket.
//!
//! Plan of record for the implementation:
//!
//! * **Inbound** — gateway `HELLO`/`IDENTIFY`/`HEARTBEAT`/`RESUME` with the
//!   `GUILD_MESSAGES`, `DIRECT_MESSAGES` and `MESSAGE_CONTENT` intents; the
//!   heartbeat interval comes from `HELLO`, never from a constant.
//! * **Outbound** — create then edit a message for progressive output, split to
//!   Discord's 2000-character limit without cutting a code block in half.
//! * **Addressing** — a guild message counts as addressed when the bot appears in
//!   `mentions`; direct messages always count.
//! * **Dedup** — message ids, plus ignoring our own messages.
//! * **Errors** — `429` and the `X-RateLimit-*` bucket headers must be honoured,
//!   including the `global` flag; a failed `RESUME` re-`IDENTIFY`s and backfills.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "discord",
    display_name: "Discord",
    description: "Discord bot: gateway websocket, streamed replies, guild mention gate.",
    docs: "docs/guide/channels-discord.md",
    maturity: Maturity::Planned,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 2000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "bot_token": "",
  "guild_allowlist": [],
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true,
  "streaming": true
}"#,
    requires: &["a Discord application bot token with the message content intent"],
};

planned_provider!(DEFINITION);

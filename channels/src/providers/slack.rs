//! Slack: Socket Mode (preferred) or the Events API.
//!
//! Plan of record for the implementation:
//!
//! * **Inbound** — `apps.connections.open` to obtain a WSS URL, then envelope
//!   acknowledgement; every envelope must be acked or Slack redelivers it.
//! * **Outbound** — `chat.postMessage` / `chat.update`, with `thread_ts` for
//!   threaded replies and `reactions.add` as a cheap acknowledgement.
//! * **Limits** — Slack counts UTF-16 code units and rejects over 4000 per
//!   message, so chunking must use [`LengthUnit::Utf16`].
//! * **Dedup** — `event_id` plus `client_msg_id`, and `subtype` filtering so a
//!   bot's own message edits do not loop back as prompts.
//! * **Errors** — `429` carries `Retry-After`; `invalid_auth` is permanent.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "slack",
    display_name: "Slack",
    description: "Slack app: Socket Mode or Events API, threaded replies, progress reactions.",
    docs: "docs/guide/channels-slack.md",
    maturity: Maturity::Planned,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: false,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Utf16,
    config_example: r#"{
  "enabled": true,
  "bot_token": "",
  "app_token": "",
  "signing_secret": "",
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true,
  "streaming": true
}"#,
    requires: &["a Slack app with Socket Mode enabled"],
};

planned_provider!(DEFINITION);

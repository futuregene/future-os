//! Mattermost: self-hosted teams, REST v4 plus its websocket event stream.
//!
//! Plan of record for the implementation:
//!
//! * **Inbound** — `/api/v4/websocket` with the `posted` event; reconnect with
//!   backoff and re-`hello` on every reconnect.
//! * **Outbound** — create a post, then update it for progressive output;
//!   `root_id` carries a threaded reply.
//! * **Addressing** — `data.mentions` contains the bot's user id; the channel
//!   allowlist gates whole channels.
//! * **Errors** — `401`/`403` are permanent, `429`/`5xx` retryable.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "mattermost",
    display_name: "Mattermost",
    description: "Mattermost server: websocket events, threaded posts, channel allowlist.",
    docs: "docs/guide/channels-mattermost.md",
    maturity: Maturity::Planned,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: false,
        reactions: true,
        media_in: false,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "base_url": "https://mattermost.example.com",
  "token": "",
  "channel_allowlist": [],
  "require_mention": true,
  "streaming": true
}"#,
    requires: &["a Mattermost bot or personal access token"],
};

planned_provider!(DEFINITION);

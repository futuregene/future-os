//! Signal: a chat client driven through a local `signal-cli` daemon.
//!
//! Signal has no bot API, so the supported shape is a `signal-cli` instance
//! the user runs themselves; this channel is a client of its HTTP interface.
//! The requirement is declared in the definition so nobody has to guess.
//!
//! Plan of record for the implementation:
//!
//! * **Inbound** — poll `/v1/receive/<number>` for envelopes, distinguishing a
//!   direct message from a group (`groupId`), and translate attachments into
//!   [`crate::bridge::MediaRef`].
//! * **Outbound** — `/v2/send` with `recipient` or `group-id`.
//! * **Addressing** — group messages require a mention unless configured
//!   otherwise.
//! * **Errors** — a connection failure is retryable; `unregistered`/
//!   `not registered` is permanent.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "signal",
    display_name: "Signal",
    description: "Signal via a local signal-cli daemon (HTTP interface).",
    docs: "docs/guide/channels-signal.md",
    maturity: Maturity::Planned,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "http_url": "http://127.0.0.1:8080",
  "number": "+15550001234",
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true
}"#,
    requires: &["a running signal-cli daemon with a registered number"],
};

planned_provider!(DEFINITION);

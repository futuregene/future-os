//! Linq: a hosted iMessage/RCS messaging API.
//!
//! Marked preview rather than live: the API surface used here follows the
//! public documentation, but the deployment-specific details (which webhook
//! signature header a given account receives, and the exact sender format)
//! vary, so treat a fresh setup as needing verification.
//!
//! Plan of record for the implementation:
//!
//! * **Inbound** — webhook through [`crate::transport::webhook`], signature
//!   verified before parsing.
//! * **Outbound** — REST send with the configured sender, split to the
//!   platform limit and deduplicated by conversation.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "linq",
    display_name: "Linq (iMessage / RCS)",
    description: "Hosted iMessage and RCS messaging: signed webhook inbound, REST outbound.",
    docs: "docs/guide/channels-linq.md",
    maturity: Maturity::Planned,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: false,
        media_in: false,
        media_out: false,
        mention_gate: false,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "api_key": "",
  "from": "",
  "webhook": { "addr": "127.0.0.1:8789", "path": "/webhooks/linq" },
  "sender_allowlist": []
}"#,
    requires: &["a Linq account with a provisioned sender"],
};

planned_provider!(DEFINITION);

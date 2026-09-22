//! WhatsApp: the official Cloud API.
//!
//! The Cloud API is webhook-driven: Meta verifies the endpoint once with a
//! `hub.challenge` echo, then POSTs signed message events. Sending is only
//! allowed inside the 24-hour customer service window unless a template is used,
//! which is why a failed send here can legitimately be permanent.
//!
//! Plan of record for the implementation:
//!
//! * **Inbound** — serve `/webhooks/whatsapp` through
//!   [`crate::transport::webhook`], answer the verification GET, and verify
//!   `X-Hub-Signature-256` against the raw body before trusting anything.
//! * **Outbound** — `POST /{phone_number_id}/messages`, split to 4096, and
//!   `status: read` acknowledgements.
//! * **Errors** — a closed service window and an invalid token are permanent;
//!   rate limits are not.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "whatsapp",
    display_name: "WhatsApp (Cloud API)",
    description: "WhatsApp Business Cloud API: verified webhook inbound, Graph API outbound.",
    docs: "docs/guide/channels-whatsapp.md",
    maturity: Maturity::Planned,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: false,
    },
    max_text_len: 4096,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "phone_number_id": "",
  "access_token": "",
  "verify_token": "",
  "app_secret": "",
  "webhook": { "addr": "127.0.0.1:8788", "path": "/webhooks/whatsapp" },
  "sender_allowlist": []
}"#,
    requires: &["a Meta WhatsApp Business app and a publicly reachable webhook URL"],
};

planned_provider!(DEFINITION);

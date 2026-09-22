//! QQ: the official bot platform, REST plus a websocket gateway.
//!
//! Plan of record for the implementation:
//!
//! * **Auth** — an app access token from `app_id`/`app_secret`, refreshed with
//!   margin before it expires (the platform returns an absolute lifetime).
//! * **Inbound** — the gateway handshake: `op 10` hello (heartbeat interval),
//!   `op 2` identify, `op 1` heartbeat, `op 11` ack, `op 0` dispatch carrying
//!   `C2C_MESSAGE_CREATE` and `GROUP_AT_MESSAGE_CREATE`.
//! * **Outbound** — reply with the originating `msg_id` and a per-message
//!   `msg_seq`, so a multi-chunk answer stays one reply.
//! * **Reconnect** — `op 7`/`op 9` mean re-identify, not backoff-and-hope.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "qq",
    display_name: "QQ",
    description: "QQ bot (open platform v2): gateway events, C2C and group replies.",
    docs: "docs/guide/channels-qq.md",
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
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "app_id": "",
  "app_secret": "",
  "sandbox": false,
  "group_allowlist": [],
  "require_mention": true
}"#,
    requires: &["a QQ open-platform bot"],
};

planned_provider!(DEFINITION);

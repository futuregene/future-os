//! iMessage on macOS, through Messages.app and the local message database.
//!
//! There is no iMessage API: sending goes through AppleScript and receiving
//! reads the local Messages database. Both are macOS-only, so the channel is
//! compiled behind `cfg(target_os = "macos")` and reports a clear
//! unsupported error elsewhere rather than pretending to exist.
//!
//! Plan of record for the implementation:
//!
//! * **Outbound** — `osascript` driving Messages.app. Every interpolated value
//!   must be escaped: a message body is attacker-controlled text, and an
//!   unescaped quote would let it become AppleScript source.
//! * **Inbound** — poll `~/Library/Messages/chat.db` read-only via the system
//!   `sqlite3`, converting Apple's epoch (2001-01-01, nanoseconds) into Unix
//!   milliseconds. Full Disk Access is required, and a missing permission must
//!   produce an actionable error, not silence.
//! * **Addressing** — direct conversations only; a sender allowlist gates them.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "imessage",
    display_name: "iMessage (macOS)",
    description: "macOS Messages.app: AppleScript send, local database receive.",
    docs: "docs/guide/channels-imessage.md",
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
    max_text_len: 20000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "recipients": [],
  "sender_allowlist": [],
  "poll_seconds": 5,
  "db_path": ""
}"#,
    requires: &[
        "macOS",
        "Full Disk Access for the terminal running the bridge",
    ],
};

planned_provider!(DEFINITION);

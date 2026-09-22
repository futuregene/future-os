//! IRC: the line protocol itself, over TCP or TLS.
//!
//! IRC is the cheapest channel to support well because it is plain text, but it
//! is also where the details bite: a message cap measured in *bytes including
//! the protocol overhead*, a nickname collision on connect, and servers that
//! silently drop an idle client.
//!
//! Plan of record for the implementation:
//!
//! * **Connection** — `CAP` negotiation with `SASL PLAIN` when credentials are
//!   configured, `NICK`/`USER` registration, `PING`/`PONG` keepalive, and
//!   automatic reconnect with backoff.
//! * **Inbound** — parse `PRIVMSG` into [`crate::bridge::Inbound`], resolving
//!   `nick!user@host` to the nickname, and treating a channel message as
//!   addressed when the nickname is prefixed.
//! * **Outbound** — split so the *entire* line stays inside 512 bytes, which
//!   means the limit is the platform limit minus the command overhead.
//! * **Noise control** — ignore CTCP (`ACTION`, `VERSION`) and our own echoes.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "irc",
    display_name: "IRC",
    description:
        "IRC over TCP or TLS: SASL authentication, channel allowlist, byte-capped messages.",
    docs: "docs/guide/channels-irc.md",
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
    // The 512-byte protocol cap minus the PRIVMSG envelope; the provider
    // subtracts its own overhead again when addressing a channel.
    max_text_len: 400,
    length_unit: LengthUnit::Bytes,
    config_example: r##"{
  "enabled": true,
  "server": "irc.libera.chat",
  "port": 6697,
  "tls": true,
  "nick": "",
  "sasl": { "account": "", "password": "" },
  "channels": ["#future"],
  "require_mention": true
}"##,
    requires: &[],
};

planned_provider!(DEFINITION);

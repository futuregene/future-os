//! Email: SMTP submission and IMAP retrieval.
//!
//! Implemented directly against the two protocols rather than through a mail
//! crate, because the subset a chat bridge needs is small — submit a message,
//! read unseen ones — and the protocols are stable, text-based, and testable
//! against a scripted session.
//!
//! Plan of record for the implementation:
//!
//! * **Send** — `EHLO`, `STARTTLS` (or implicit TLS on 465), `AUTH PLAIN` or
//!   `AUTH LOGIN`, `MAIL FROM`/`RCPT TO`/`DATA`, with dot-stuffing and CRLF
//!   normalisation. The envelope sender must match the configured address.
//! * **Receive** — `LOGIN`, `SELECT`, `UID SEARCH UNSEEN`, `UID FETCH
//!   BODY.PEEK[]`, then mark seen; polling rather than `IDLE` keeps one code
//!   path for every server.
//! * **Parsing** — `multipart/alternative` and `mixed`, quoted-printable, base64
//!   bodies, RFC 2047 encoded headers, preferring `text/plain` and falling back
//!   to `text/html` with tags stripped. Attachments are recognized but not
//!   downloaded: a mailbox is not a file transfer, and the sender can resend.
//! * **Loop safety** — never answer our own address, honour a subject prefix
//!   filter, and thread replies with `In-Reply-To`/`References`.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::providers::unsupported::planned_provider;
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "email",
    display_name: "Email (IMAP + SMTP)",
    description: "Mailbox channel: IMAP polling for mail, SMTP submission for replies.",
    docs: "docs/guide/channels-email.md",
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
    max_text_len: 100_000,
    length_unit: LengthUnit::Bytes,
    config_example: r#"{
  "enabled": true,
  "imap": { "host": "", "port": 993, "username": "", "password": "", "mailbox": "INBOX" },
  "smtp": { "host": "", "port": 587, "username": "", "password": "", "from": "" },
  "poll_seconds": 30,
  "sender_allowlist": [],
  "subject_prefix": ""
}"#,
    requires: &["a mailbox that allows IMAP and SMTP with a password or app password"],
};

planned_provider!(DEFINITION);

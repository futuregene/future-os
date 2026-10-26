//! Application-independent Remote wire contract.

mod command_generated;
pub mod files;
pub mod handshake;
pub mod invitation;
pub mod json;
pub mod jwt;
pub use command_generated::{IncomingCmd, UploadReference};
pub use future_remote_crypto as crypto;

/// Clients that declare this capability implement lean event/history reads.
pub const SEGMENTED_CONTENT_FEATURE: &str = "segmented_content_v1";
pub const LEAN_EVENTS_FEATURE: &str = "lean_events_v1";

pub const EVENT_COALESCING_FEATURE: &str = "event_coalescing_v1";
pub const CATALOG_ACTIVITY_FEATURE: &str = "catalog_activity_v1";
pub const REPLY_GZIP_FEATURE: &str = "reply_gzip_v1";

/// Clients that declare this capability keep proving they are still there:
/// a `client_presence` beat while they are, and `client_disconnect` when they
/// leave. A client that does not declare it is never presumed gone for being
/// silent — it never promised to speak.
pub const CLIENT_PRESENCE_FEATURE: &str = "client_presence_v1";

/// Capability a client declares on `secure_ready` to receive a paged read's
/// chunk bytes as raw bytes instead of base64 inside a JSON string.
///
/// The paging transport asks for the page it already asks for; what changes is
/// only how one chunk is *carried*. Today a chunk's bytes are base64url-encoded
/// into `readChunk.data` before the reply is JSON-serialized and gzipped, so the
/// compressor is fed a 4/3-expanded alphabet it cannot model: on this repo's
/// 0.5 MiB reference page that costs 44% of the encoded reply (see
/// `docs/testing/remote-page-encoding-2026-10-26.md`). The wire is already
/// binary-safe below the JSON layer — the reply is sealed and published as
/// bytes — so the expansion buys nothing here; it exists because a JSON string
/// is the only way to put bytes *inside* a JSON document.
///
/// The frame ([`json::encode_binary_frame`]) keeps the JSON header and appends
/// the chunk's bytes behind it, so every check the reader makes about `id`,
/// `totalBytes`, `offset` and `nextOffset` is unchanged: only `data`'s carrier
/// moved. gzip still applies to the whole frame — plain JSON text compresses
/// well, and dropping it would trade this win for a larger one lost.
///
/// Negotiated, both ways, for the same reason `selective_events_v1` is:
///
/// * a host that does not know the flag ignores the declaration and keeps
///   sending base64 pages, which this client still reads (its decoder is
///   shape-driven, see `json::split_binary_frame`);
/// * this client declares it only when the host's confirmation advertises it,
///   so a host that knows the flag but chose not to offer it for this pairing
///   never sees the declaration and never sends a frame.
///
/// A client that never declares it keeps the legacy page byte for byte,
/// including the base64 `data` field and the size assertions built on it.
pub const READ_BINARY_FEATURE: &str = "read_binary_v1";

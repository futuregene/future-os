//! Application-independent Remote wire contract.

mod command_generated;
pub mod diagnostics;
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
/// A client that applies *row-level* catalogue deltas.
///
/// Declaring it promises something the wire depends on: a `state.sessions`
/// packet may then carry `full: false` with only the rows that changed and the
/// ids that left, and the client is the half that keeps the rest. A client that
/// does not declare it keeps receiving whole snapshots, byte for byte as before
/// — reading a delta as a snapshot would make every absent row look deleted.
pub const CATALOG_DELTA_FEATURE: &str = "catalog_delta_v1";

/// A client that reads the catalogue as a *window* of the newest conversations
/// and can ask for the next page.
///
/// Declaring it promises the other half of the deal: the host then sends the
/// newest `CATALOG_WINDOW_ROOTS` conversation trees (plus the pinned ones)
/// instead of every conversation it owns, and the reader offers the rest through
/// `list_sessions` with a `cursor`. A client that does not declare it — the
/// phone, and every build older than this one — keeps receiving the whole list,
/// byte for byte as before: a window it did not ask for would look like a
/// machine that lost most of its conversations, with no way to ask for them.
pub const CATALOG_WINDOW_FEATURE: &str = "catalog_window_v1";

/// How many conversation roots one windowed catalogue page carries.
///
/// The wire-level number both ends use: the host sends this many of the newest
/// conversation trees in a `state.sessions` window, and a client asks
/// `list_sessions` for the next page of the same size. Fifty roots is several
/// screens of the newest conversations, and about a twelfth of what a machine
/// with 604 conversations used to send on every connect.
pub const CATALOG_WINDOW_ROOTS: i64 = 50;
pub const REPLY_GZIP_FEATURE: &str = "reply_gzip_v1";

/// Whether a host serves the embedded terminal over the control plane.
///
/// Advertised by the *host*, not declared by a client: a paired desktop shows
/// the terminal entry point only on a host that says it can open a shell. Both
/// entry points install the registry (`future_host::terminal::install_service`)
/// — the window and `future host` on a box with no GUI — so what a client sees
/// is a property of the build and of the process, not of the way the machine
/// was started. A host that never installed one leaves the capability out, and
/// the client hides the panel instead of collecting `Unsupported command` on
/// every keystroke.
///
/// Terminal commands travel the ordinary command lane (list/create/update/
/// delete/read/write), which is why a client that never sends them needs no
/// change at all. Nothing about the lane is binary: a read page and a keystroke
/// are base64 in the same JSON envelope every other command uses.
pub const TERMINAL_FEATURE: &str = "terminal_v1";

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

/// Capability a host advertises when it can answer `get_reasoning` — the
/// on-demand read of one reasoning block's text.
///
/// The lean lanes send a reasoning row's boundaries and not its body (the
/// largest thing a reply carries that nobody has opened yet), so a client that
/// declares the trimmed feed draws the row from its boundary and fetches the
/// text when the reader opens it. A host that predates the command would answer
/// the fetch with "Unsupported command", i.e. a row that promises content and
/// produces an error, so the row only offers to open when this is advertised.
pub const REASONING_FETCH_FEATURE: &str = "reasoning_fetch_v1";

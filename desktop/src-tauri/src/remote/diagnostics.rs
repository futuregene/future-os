use super::*;
pub(super) use future_remote_server::diagnostics::FailureEpisode;

pub(super) static START_EPISODE: LazyLock<FailureEpisode> = LazyLock::new(Default::default);
pub(super) static CREDENTIAL_EPISODE: LazyLock<FailureEpisode> = LazyLock::new(Default::default);

/// Port for the embedded web client HTTP server.
pub(crate) const WEB_PORT: u16 = 8022;
/// The remote browser client is a single self-contained HTML file. Embed it so
/// release bundles do not depend on the build machine's source checkout.
pub(super) const EMBEDDED_WEB_INDEX: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/index.html"));
/// A shutdown notification is advisory: never delay closing the desktop for a
/// slow or unreachable broker, but give a healthy connection a short window to
/// flush the packet before its tasks are torn down.
pub(super) const DISCONNECT_NOTICE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(750);

/// Bound on the event publish queue; on overflow the newest event is dropped
/// (logged) rather than blocking the agent event loop. The client recovers the
/// gap via `get_events_since` backfill on its next reattach.
pub(super) const EVENT_QUEUE_CAPACITY: usize = 4096;

#[cfg(test)]
pub(super) use future_remote_server::diagnostics::DropCounters;

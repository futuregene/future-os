#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::Ordering;

/// Port for the embedded web client HTTP server.
pub(crate) const WEB_PORT: u16 = 8022;

/// The port the web-client listener asks for.
///
/// Production keeps the fixed number: the desktop prints it, a phone on the LAN
/// reaches it, and stability is the point. Tests must not. The listener is
/// process-wide state and one test binary starts several bridges, so a fixed
/// port turns the test schedule into a collision whenever two of them overlap --
/// a failure that says nothing about the product. There the OS picks (`0`) and
/// the listener's own `local_addr()` supplies the number the status reports; a
/// test that needs a port already *taken* pins one through [`pin_web_port`].
///
/// The `test-support` feature alone does not reach the ephemeral branch, which
/// matters because the desktop and CLI both enable it: fixture mode is only ever
/// set by `#[cfg(test)]` code (`configure_entry_fixture`, `HomeGuard`), so a
/// shipped binary keeps [`WEB_PORT`].
pub(super) fn web_bind_port() -> u16 {
    #[cfg(any(test, feature = "test-support"))]
    {
        if crate::remote::fixture_mode() {
            return FIXTURE_WEB_PORT.load(Ordering::Relaxed);
        }
    }
    WEB_PORT
}

#[cfg(any(test, feature = "test-support"))]
static FIXTURE_WEB_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

/// Hold the web listener on `port` for as long as the guard lives; `0` (the
/// default) means the OS picks. Fixture-only: the bind-failure test needs a port
/// somebody else holds, and with an ephemeral port it would have to guess.
///
/// The fixture tests that call this hold the home guard, which serializes them,
/// so one process-wide value is enough -- and dropping the guard restores the
/// default even when the test panics.
#[cfg(any(test, feature = "test-support"))]
pub(super) struct WebPortPin;

#[cfg(any(test, feature = "test-support"))]
pub(super) fn pin_web_port(port: u16) -> WebPortPin {
    FIXTURE_WEB_PORT.store(port, Ordering::Relaxed);
    WebPortPin
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for WebPortPin {
    fn drop(&mut self) {
        FIXTURE_WEB_PORT.store(0, Ordering::Relaxed);
    }
}

/// The remote browser client is a single self-contained HTML file. Embed it so
/// release bundles do not depend on the build machine's source checkout.
pub(super) const EMBEDDED_WEB_INDEX: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/index.html"));
/// Bound on the event publish queue; on overflow the newest event is dropped
/// (logged) rather than blocking the agent event loop. The client recovers the
/// gap via `get_events_since` backfill on its next reattach.
#[cfg(any(test, feature = "test-support"))]
pub(super) const EVENT_QUEUE_CAPACITY: usize = 4096;

#[cfg(test)]
pub(super) use future_remote_server::diagnostics::DropCounters;

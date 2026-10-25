//! Measurement tests for the drain's admission sub-steps.
//!
//! These sit inside `service::shutdown` rather than next to `service::tests` for
//! one reason: the warning they assert on is emitted on whichever thread runs the
//! drain, and a thread-local subscriber only observes that when the drain runs on
//! the test's own thread. Driving `drain` directly is what makes the assertion
//! deterministic instead of racing the supervisor thread.

use super::*;
use std::sync::{Arc, Mutex};

/// A `MakeWriter` that keeps everything the subscriber formats.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|error| error.into_inner()))
            .into_owned()
    }
}

/// One live session, carrying the service-work counter the drain waits on.
fn fixture() -> AgentHandle {
    let state = crate::rpc::service_test_state();
    for session in state.sessions.read().values() {
        session.write().service_work = state.service_work.clone();
    }
    AgentHandle {
        state,
        shutdown: Arc::new(ShutdownShared::default()),
    }
}

/// A session whose write lock is held too long is named, with its sub-step.
///
/// This is the case a bare "admission" stage could not describe: the drain spent
/// its time on ONE session's lock, not on the service being wedged. The warning
/// names the sub-step that was waiting, the session, and how long the wait was,
/// so the next timeout report can be read instead of guessed at.
#[test]
fn a_held_session_lock_names_the_sub_step_and_the_session() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    let session_id = tracing::subscriber::with_default(subscriber, || {
        let handle = fixture();
        let (session_id, session) = {
            let sessions = handle.state.sessions.read();
            let (id, handle) = sessions
                .iter()
                .next()
                .expect("the fixture owns one live session");
            (id.clone(), Arc::clone(handle))
        };
        // Hold the session's write lock past the threshold, and say so instead of
        // sleeping and hoping: the drain must start against a lock already taken.
        let (acquired, taken) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _held = session.write();
            acquired.send(()).unwrap();
            std::thread::sleep(SLOW_SHUTDOWN_STEP * 4);
        });
        taken.recv().expect("the holder takes the lock first");

        handle.drain();
        holder.join().unwrap();
        session_id
    });

    let log = capture.text();
    assert!(
        log.contains(STAGE_INTERRUPTING_SESSIONS),
        "the blocked sub-step must be named in the warning: {log}"
    );
    assert!(
        log.contains(&session_id),
        "the session that held the lock must be named ({session_id}): {log}"
    );
    assert!(
        log.contains("Agent shutdown step blocked"),
        "the warning must be recognisable: {log}"
    );
    // The snapshot itself was not contended here, so claiming it would make the
    // warning useless for telling the two waits apart.
    assert!(
        !log.contains(STAGE_SESSION_SNAPSHOT),
        "an uncontended snapshot must not be reported: {log}"
    );
}

/// An uncontended drain warns about nothing.
///
/// The threshold has to be a filter, not a formality: a warning on every ordinary
/// shutdown would bury the one that matters.
#[test]
fn a_drain_with_free_locks_warns_about_no_step() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let handle = fixture();
        handle.drain();
    });
    let log = capture.text();
    assert!(
        !log.contains("Agent shutdown step blocked"),
        "a fast drain must stay quiet: {log}"
    );
}

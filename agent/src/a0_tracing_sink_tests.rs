//! The process-wide `tracing` sink the unit suite installs
//! (`test_support::tracing_sink`).
//!
//! `tracing` evaluates an event's field arguments only when the callsite is not
//! `Interest::never()` *and* an enabled dispatcher is reachable from the
//! emitting thread. Without a sink every callsite caches `never`, so the
//! argument regions of `tracing::info!(bytes = body.len(), …)` stay dark under
//! llvm-cov even though the statement runs. These tests pin the three
//! properties that make the sink usable as measurement rather than decoration:
//! it receives events from threads it was not installed on, it enables the
//! lowest level, and its own `Write` accounting is real.

use std::sync::Mutex;

use super::test_support::{tracing_sink, TracingSink, PROBE_MARKER};

/// The probe flag is one bit on a sink every test shares, so the two tests that
/// arm it take turns; without the lock they would clear and read each other's
/// marker and flake under the default parallel harness.
static PROBE_LOCK: Mutex<()> = Mutex::new(());

fn probe_lock() -> std::sync::MutexGuard<'static, ()> {
    PROBE_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// The sink sees an event emitted on a **spawned** thread.
///
/// This is the property a thread-local `with_default` subscriber lacks: it
/// would see only the events emitted on the thread that installed it, which is
/// where the field arguments of background work (run loop, journal drain,
/// scheduler) live.
#[test]
fn an_event_from_a_spawned_thread_reaches_the_process_wide_sink() {
    let _turn = probe_lock();
    let sink = tracing_sink();
    sink.clear_probe();
    let writes_before = sink.writes();
    let message = format!("{PROBE_MARKER} from a spawned thread");
    let worker = std::thread::spawn(move || {
        let queue_depth = 3;
        tracing::info!(queue_depth, message = %message, "coverage sink probe");
    });
    worker.join().expect("probe thread");
    let writes_after = sink.writes();
    assert!(
        sink.probe(),
        "the global sink recorded no probe event ({writes_before} -> {writes_after} writes)"
    );
    assert!(
        writes_after > writes_before,
        "the probe event did not reach the sink's Write ({writes_before} -> {writes_after})"
    );
}

/// The sink enables `TRACE`, the lowest level, so no callsite is silently
/// filtered out by level before its arguments are evaluated.
#[test]
fn the_sink_enables_the_lowest_level() {
    let _turn = probe_lock();
    let sink = tracing_sink();
    sink.clear_probe();
    tracing::trace!(message = PROBE_MARKER, "trace coverage sink probe");
    assert!(
        sink.probe(),
        "a TRACE event was dropped before reaching the sink"
    );
}

/// The sink's `Write` books the bytes it is handed and flushes without
/// emitting: measured on a private sink, because the shared one also carries
/// the whole suite's traffic.
#[test]
fn the_sink_write_accounts_bytes_and_flushes_quietly() {
    let sink = TracingSink::default();
    let mut writer = sink.writer();
    assert_eq!((sink.bytes(), sink.writes()), (0, 0));

    let payload = format!("prefix {PROBE_MARKER} suffix");
    let written =
        std::io::Write::write(&mut writer, payload.as_bytes()).expect("sink accepts writes");
    assert_eq!(written, payload.len(), "the sink reports a short write");
    assert_eq!(sink.bytes(), payload.len());
    assert_eq!(sink.writes(), 1);
    assert!(sink.probe(), "the marker inside one chunk was not detected");

    std::io::Write::flush(&mut writer).expect("the sink flushes");
    assert_eq!(sink.flushes(), 1);
    assert_eq!(
        (sink.bytes(), sink.writes()),
        (payload.len(), 1),
        "a flush must not be counted as a write"
    );
}

/// A chunk that does not carry the marker leaves the probe disarmed: the flag
/// is evidence about one event, not a latch that any traffic can trip.
#[test]
fn a_chunk_without_the_marker_leaves_the_probe_disarmed() {
    let sink = TracingSink::default();
    let mut writer = sink.writer();
    std::io::Write::write(&mut writer, b"unrelated diagnostic text").expect("sink accepts writes");
    assert!(!sink.probe(), "an unrelated chunk armed the probe");
    assert_eq!(sink.bytes(), "unrelated diagnostic text".len());
}

/// A chunk shorter than the marker is not a false positive: the scan has to
/// bound itself by the chunk, not by the marker.
#[test]
fn a_chunk_shorter_than_the_marker_is_not_scanned_out_of_bounds() {
    let sink = TracingSink::default();
    let mut writer = sink.writer();
    let partial = &PROBE_MARKER.as_bytes()[..4];
    assert_eq!(
        std::io::Write::write(&mut writer, partial).expect("sink accepts writes"),
        partial.len()
    );
    assert!(!sink.probe(), "a truncated marker armed the probe");
}

/// `tracing_sink()` is process-wide: every call hands back the same counters,
/// so a test's probe cannot be satisfied by a second, private sink.
#[test]
fn every_caller_reaches_the_one_process_wide_sink() {
    let first = tracing_sink();
    let second = tracing_sink();
    assert!(
        first.is_same_sink(&second),
        "tracing_sink() handed out two different sinks"
    );
    let mut writer = second.writer();
    let bytes_before = first.bytes();
    std::io::Write::write(&mut writer, b"shared").expect("sink accepts writes");
    assert!(
        first.bytes() >= bytes_before + "shared".len(),
        "a write through one handle is invisible to the other ({bytes_before} -> {})",
        first.bytes()
    );
}

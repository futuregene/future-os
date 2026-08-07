//! End-to-end smoke test for the POSIX terminal backend.
//!
//! Exercises `Terminal::start` (raw mode, alt screen, bracketed paste, signal
//! handlers, reader thread) under a real PTY. Run with:
//!
//!   cargo run -p future-tui --example smoke
//!
//! and pipe input, e.g.:
//!
//!   printf '\x1b[A\x03' | script -q /tmp/smoke.out target/debug/examples/smoke
//!
//! The transcript should contain: READY, then `key=up` / `key=ctrl+c`, then
//! `resizes=N`, then STOPPED.

use std::sync::mpsc;
use std::time::Duration;

use future_tui::keys;
use future_tui::terminal::Terminal;

fn main() {
    let (tx, rx) = mpsc::channel::<String>();
    let (rtx, rrx) = mpsc::channel::<()>();

    let mut term = Terminal::new().expect("Terminal::new");
    term.start(
        Box::new(move |data: String| {
            if let Some(k) = keys::parse_key(&data) {
                let _ = tx.send(format!("key={k}"));
            }
        }),
        Box::new(move || {
            let _ = rtx.send(());
        }),
    )
    .expect("Terminal::start (needs a TTY)");

    term.write("READY\n");

    // Collect input for a moment, then stop.
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    let mut keys_received: Vec<String> = Vec::new();
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(k) => keys_received.push(k),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let mut resizes = 0usize;
    while let Ok(()) = rrx.try_recv() {
        resizes += 1;
    }

    for k in &keys_received {
        term.write(&format!("{k}\n"));
    }
    term.write(&format!("resizes={resizes}\n"));

    term.stop();
    term.write("STOPPED\n");
}

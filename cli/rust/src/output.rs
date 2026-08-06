//! Output sinks for the CLI.
//!
//! The TypeScript CLI writes to process stdout/stderr via `console.log` /
//! `console.error` (each call appends a trailing newline). This module
//! reproduces that contract behind an injectable sink so the same dispatch
//! code can run against the real stdio (the `future` binary) or in-memory
//! buffers (golden/diff tests comparing byte-for-byte against the TS CLI).

use std::io::Write;
use std::sync::{Arc, Mutex};

/// A shared write sink for stdout. `Send` so it can be used across `.await`
/// points inside async command implementations.
#[derive(Clone)]
pub struct Output {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
    err: Arc<Mutex<Box<dyn Write + Send>>>,
}

/// `Write` adapter over `Arc<Mutex<Vec<u8>>>` for capture mode.
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Captured stdout/stderr buffers returned by [`Output::memory`], for tests
/// that need to inspect what a dispatch produced.
pub struct Captured {
    pub out: Arc<Mutex<Vec<u8>>>,
    pub err: Arc<Mutex<Vec<u8>>>,
}

impl Output {
    /// Sink that writes to the real process stdout/stderr.
    pub fn stdio() -> Self {
        Self {
            out: Arc::new(Mutex::new(Box::new(std::io::stdout()))),
            err: Arc::new(Mutex::new(Box::new(std::io::stderr()))),
        }
    }

    /// Sink that captures into in-memory buffers.
    pub fn memory() -> (Self, Captured) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::new(Mutex::new(Vec::new()));
        let output = Self {
            out: Arc::new(Mutex::new(Box::new(SharedBuf(out.clone())))),
            err: Arc::new(Mutex::new(Box::new(SharedBuf(err.clone())))),
        };
        (output, Captured { out, err })
    }

    /// `console.log` equivalent: writes the line plus a trailing newline.
    pub fn log(&self, msg: &str) {
        writeln!(self.out.lock().expect("stdout sink poisoned"), "{msg}").ok();
    }

    /// `console.error` equivalent: writes the line plus a trailing newline.
    pub fn log_err(&self, msg: &str) {
        writeln!(self.err.lock().expect("stderr sink poisoned"), "{msg}").ok();
    }

    /// `process.stdout.write` equivalent: raw bytes, no newline appended.
    pub fn write_out(&self, s: &str) {
        write!(self.out.lock().expect("stdout sink poisoned"), "{s}").ok();
    }

    /// `process.stderr.write` equivalent: raw bytes, no newline appended.
    pub fn write_err(&self, s: &str) {
        write!(self.err.lock().expect("stderr sink poisoned"), "{s}").ok();
    }

    /// Flush both sinks (a no-op for captured buffers).
    pub fn flush(&self) {
        let _ = self.out.lock().expect("stdout sink poisoned").flush();
        let _ = self.err.lock().expect("stderr sink poisoned").flush();
    }
}

//! The embedded terminal as a host service.
//!
//! The PTY registry belongs to the entry point that owns the shells — the
//! Desktop window owns one (`desktop/src-tauri/src/terminal`), a headless host
//! does not — while the remote control plane that has to reach it lives in this
//! crate, which must not depend on a GUI. So the two meet here: an entry point
//! installs an implementation once at startup, and everything above it
//! (`host::remote_host::business::terminal`) talks to the trait.
//!
//! Nothing here owns state. A process either installed a service or it did not,
//! and that answer also decides whether the handshake advertises
//! [`future_remote_protocol::TERMINAL_FEATURE`] — the capability a paired
//! desktop reads before it offers the panel at all.
//!
//! The shapes mirror the loopback HTTP routes the local webview already speaks
//! (`docs/internals/desktop/embedded-terminal.md`), including the stable
//! `TERMINAL_*` error codes, because both transports feed the same client code.

use serde::Serialize;
use std::sync::{Arc, OnceLock};

/// A terminal session as the client sees it. Never contains output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    /// The host's own conversation id the shell is scoped to.
    pub thread_id: String,
    pub title: String,
    /// The directory the shell was started in, after resolution. The user asked
    /// for a shell "here", so "here" travels back with it.
    pub cwd: String,
    /// `running` | `exited`.
    pub status: String,
    pub exit_code: Option<i32>,
    pub cols: u16,
    pub rows: u16,
}

/// One page of retained output, and the cursor to ask from next time.
///
/// Absolute byte offsets, exactly like the loopback transport: a client that
/// reconnects asks for the bytes after what it already applied and is told when
/// the retained tail no longer covers that (`start > cursor requested`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPage {
    pub bytes: Vec<u8>,
    /// Absolute end offset after these bytes.
    pub cursor: u64,
    /// Absolute offset of the first returned byte.
    pub start: u64,
    pub status: String,
    pub exit_code: Option<i32>,
}

/// A refusal, carrying the same stable code the loopback routes use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalError {
    /// No such session on this host: evicted, or a tab from an older run.
    NotFound,
    /// The conversation exists but accepts no shells (deleted, read-only).
    ThreadNotWritable,
    /// The shell has exited; a write is not an error the user must see.
    Closed,
    /// Nothing usable to spawn: no shell, no directory, or a failed spawn.
    Unusable(String),
    /// Capacity limit reached; retry after closing a tab.
    Capacity(String),
    /// This host serves no terminals at all.
    Unavailable,
}

impl TerminalError {
    /// Stable code for the client. Kept in step with
    /// `terminal::manager::ManagerError::code` — the two transports answer the
    /// same failure with the same word.
    pub fn code(&self) -> &'static str {
        match self {
            TerminalError::NotFound => "TERMINAL_NOT_FOUND",
            TerminalError::ThreadNotWritable => "THREAD_READONLY",
            TerminalError::Closed => "TERMINAL_CLOSED",
            TerminalError::Unusable(_) => "TERMINAL_UNAVAILABLE",
            TerminalError::Capacity(_) => "CAPACITY_EXCEEDED",
            TerminalError::Unavailable => "TERMINALS_UNSUPPORTED",
        }
    }

    /// The wire error: the code, a colon, then the sentence a client may show.
    pub fn message(&self) -> String {
        let sentence = match self {
            TerminalError::NotFound => "that terminal is no longer on this machine".to_string(),
            TerminalError::ThreadNotWritable => {
                "this conversation does not accept terminal tabs".to_string()
            }
            TerminalError::Closed => "the shell has exited".to_string(),
            // These two carry their own coded sentence: the layer that knows
            // *why* is the one that names the code, exactly as the loopback
            // routes do.
            TerminalError::Unusable(detail) | TerminalError::Capacity(detail) => {
                return detail.clone()
            }
            TerminalError::Unavailable => {
                "this machine does not serve terminals to paired devices".to_string()
            }
        };
        format!("{}: {sentence}", self.code())
    }
}

/// What a host that owns a PTY registry exposes to the control plane.
///
/// Implementations are synchronous, like the registry itself: a create spawns a
/// process and returns its id, and a read copies at most a bounded page out of
/// an in-memory buffer. Nothing here talks to the network.
pub trait TerminalService: Send + Sync {
    /// Sessions scoped to one conversation, running and retained-exited.
    fn list(&self, thread_id: &str) -> Vec<TerminalInfo>;
    /// Spawn a shell in the conversation's own directory.
    fn create(
        &self,
        thread_id: &str,
        title: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalInfo, TerminalError>;
    /// Retitle and/or resize. Resizing a dead session is not an error.
    fn update(
        &self,
        id: &str,
        title: Option<&str>,
        size: Option<(u16, u16)>,
    ) -> Result<TerminalInfo, TerminalError>;
    /// Terminate the process tree and forget the session.
    fn delete(&self, id: &str) -> Result<(), TerminalError>;
    /// Retained output after `cursor` (`-1` tails from the current end).
    fn read(&self, id: &str, cursor: i64) -> Result<TerminalPage, TerminalError>;
    /// Keyboard input, forwarded to the PTY verbatim.
    fn write(&self, id: &str, bytes: &[u8]) -> Result<(), TerminalError>;
}

static SERVICE: OnceLock<Arc<dyn TerminalService>> = OnceLock::new();

/// Install this process's terminal registry. Returns `false` when one is
/// already installed — a second entry point in one process (the Desktop's
/// window and its embedded host) must not replace the registry the first one
/// owns, because every tab the user already has lives in it.
pub fn install(service: Arc<dyn TerminalService>) -> bool {
    SERVICE.set(service).is_ok()
}

/// The installed registry, or `None` in a process that serves no terminals.
pub fn installed() -> Option<&'static Arc<dyn TerminalService>> {
    SERVICE.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_carries_its_code_threaded_through_one_stable_word() {
        assert_eq!(TerminalError::NotFound.code(), "TERMINAL_NOT_FOUND");
        assert!(TerminalError::NotFound
            .message()
            .starts_with("TERMINAL_NOT_FOUND: "));
        assert_eq!(TerminalError::Unavailable.code(), "TERMINALS_UNSUPPORTED");
        assert!(TerminalError::Unavailable
            .message()
            .starts_with("TERMINALS_UNSUPPORTED: "));
    }
}

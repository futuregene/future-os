//! This process's terminal registry, seen by the remote control plane.
//!
//! `Manager` is the whole implementation: a paired device drives the same shells
//! the person at this machine sees — in a window, or on a box running
//! `future host` with no GUI at all — scoped to the same conversations, and torn
//! down by the same rules (deleting a conversation closes its shells). Nothing
//! here duplicates state; the trait it implements
//! (`crate::terminal_service::TerminalService`) is the seam a test can replace
//! with a registry that spawns nothing.
//!
//! Two details are load-bearing:
//!
//! * **`read` attaches and detaches immediately.** The registry's replay is
//!   built from the retained tail plus an absolute cursor (`Session::attach`),
//!   which is exactly a paged read; registering a live viewer for a poll would
//!   accumulate one subscriber per request. `Attachment` has no `Drop`, so the
//!   detach below is the only thing that releases it.
//! * **Errors keep their stable codes.** The client's terminal code branches on
//!   `TERMINAL_NOT_FOUND` / `TERMINAL_CLOSED` / `CAPACITY_EXCEEDED` for the same
//!   reasons it does over loopback, so the loopback codes are what travel — an
//!   error that reached the wire as a sentence would turn a restartable tab into
//!   an unexplained failure.

use crate::terminal_service::{TerminalError, TerminalInfo, TerminalPage, TerminalService};

use super::manager::{Manager, ManagerError};
use super::session::Status;

impl TerminalService for Manager {
    fn list(&self, thread_id: &str) -> Vec<TerminalInfo> {
        self.list(Some(thread_id))
            .iter()
            .map(info)
            .collect::<Vec<_>>()
    }

    fn create(
        &self,
        thread_id: &str,
        title: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalInfo, TerminalError> {
        self.create(super::manager::CreateRequest {
            thread_id: thread_id.to_string(),
            title: title.map(str::to_string),
            // 0 means "not stated": fall back to the size every terminal client
            // has used since the feature shipped, so a poll-only client that
            // never resized still gets a usable shell.
            cols: if cols > 0 { cols } else { 80 },
            rows: if rows > 0 { rows } else { 24 },
        })
        .map(|info| self::info(&info))
        .map_err(terminal_error)
    }

    fn update(
        &self,
        id: &str,
        title: Option<&str>,
        size: Option<(u16, u16)>,
    ) -> Result<TerminalInfo, TerminalError> {
        self.update(id, title.map(str::to_string), size)
            .map(|info| self::info(&info))
            .map_err(terminal_error)
    }

    fn delete(&self, id: &str) -> Result<(), TerminalError> {
        self.remove(id).map_err(terminal_error)
    }

    fn read(&self, id: &str, cursor: i64) -> Result<TerminalPage, TerminalError> {
        let session = self.session(id).map_err(terminal_error)?;
        let mut attachment = session.attach(Some(cursor));
        let mut bytes = Vec::new();
        for chunk in &attachment.replay {
            bytes.extend_from_slice(chunk);
        }
        let page = TerminalPage {
            bytes,
            cursor: attachment.cursor,
            start: attachment.start,
            status: status(session.info().status),
            exit_code: attachment.exit_code,
        };
        // `Attachment` registers a live viewer; a paged read must release it or
        // every poll leaves one behind.
        attachment.detach();
        Ok(page)
    }

    fn write(&self, id: &str, bytes: &[u8]) -> Result<(), TerminalError> {
        let session = self.session(id).map_err(terminal_error)?;
        if !session.is_running() {
            return Err(TerminalError::Closed);
        }
        session.write(bytes).map_err(TerminalError::Unusable)
    }
}

fn info(info: &super::session::Info) -> TerminalInfo {
    TerminalInfo {
        id: info.id.clone(),
        thread_id: info.thread_id.clone(),
        title: info.title.clone(),
        cwd: info.cwd.clone(),
        status: status(info.status),
        exit_code: info.exit_code,
        cols: info.cols,
        rows: info.rows,
    }
}

fn status(status: Status) -> String {
    match status {
        Status::Running => "running".to_string(),
        Status::Exited => "exited".to_string(),
    }
}

/// The loopback transport's stable codes, carried through unchanged.
fn terminal_error(error: ManagerError) -> TerminalError {
    match error {
        ManagerError::NotFound(_) => TerminalError::NotFound,
        ManagerError::ThreadNotWritable(_) => TerminalError::ThreadNotWritable,
        ManagerError::Capacity(detail) => TerminalError::Capacity(detail),
        // Everything else already has a coded sentence (`SHELL_UNAVAILABLE:`,
        // `CWD_INVALID:`, `SPAWN_FAILED:`, `TERMINAL_STOPPING:`) that the local
        // webview renders and the remote client parses the same way.
        other => TerminalError::Unusable(format!("{}: {}", other.code(), other.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loopback_code_survives_the_hop_to_the_wire() {
        let refusal = terminal_error(ManagerError::NotFound("term_1".into()));
        assert_eq!(refusal.code(), "TERMINAL_NOT_FOUND");
        assert!(refusal.message().starts_with("TERMINAL_NOT_FOUND: "));

        let unavailable = terminal_error(ManagerError::ShellUnavailable(
            "SHELL_UNAVAILABLE: no usable shell here".into(),
        ));
        // `Unusable` carries its sentence verbatim, so the code the local
        // transport would have shown is still the first word on the wire.
        assert_eq!(
            unavailable.message(),
            "SHELL_UNAVAILABLE: SHELL_UNAVAILABLE: no usable shell here"
        );
    }

    #[test]
    fn a_capacity_refusal_keeps_its_own_variant() {
        let refusal = terminal_error(ManagerError::Capacity(
            "CAPACITY_EXCEEDED: at most 8 terminals per conversation".into(),
        ));
        assert_eq!(refusal.code(), "CAPACITY_EXCEEDED");
        assert!(refusal
            .message()
            .starts_with("CAPACITY_EXCEEDED: at most 8"));
    }
}

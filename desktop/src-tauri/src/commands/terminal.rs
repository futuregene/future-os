//! Terminal command surface.
//!
//! Deliberately one command: the webview asks where the loopback terminal
//! server is and what secret to present, then talks to it directly over HTTP
//! and WebSocket. Terminal output never crosses the Tauri IPC boundary — that
//! is the point of the transport (see `docs/internals/desktop/embedded-terminal.md`).

use crate::terminal;
use crate::AppError;

/// Where the app's own webview finds the terminal server.
///
/// Returns an error before the listener is bound; the frontend treats that as
/// "retry shortly", not as a feature that is missing.
#[tauri::command]
pub fn terminal_server_info() -> Result<terminal::ServerInfo, AppError> {
    match terminal::server::info() {
        Some(info) => Ok(info.clone()),
        None => Err(AppError::Message(
            "terminal server is not ready yet".to_string(),
        )),
    }
}

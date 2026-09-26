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

#[cfg(test)]
mod tests {
    use super::*;

    /// Marker that makes the child process below take the pre-bind branch.
    const CHILD_MARKER: &str = "FUTUREOS_TERMINAL_INFO_CHILD";

    /// The `None` arm of [`terminal_server_info`] is genuinely reachable — it is
    /// what the webview sees before the listener is bound — but not inside this
    /// test *process*: the suite shares one process and the listener test in
    /// `terminal::server` binds the process-wide server when it runs, in an
    /// order no test may depend on. Running this one test in a fresh process is
    /// what makes the pre-bind state reachable deterministically.
    #[test]
    fn terminal_server_info_reports_not_ready_before_bind() {
        if std::env::var_os(CHILD_MARKER).is_some() {
            assert!(
                terminal::server::info().is_none(),
                "the isolated process must not have bound the server"
            );
            let error = terminal_server_info().expect_err("no server is bound yet");
            assert!(
                error.to_string().contains("not ready"),
                "the pre-bind error must tell the frontend to retry: {error}"
            );
            return;
        }
        let exe = std::env::current_exe().expect("test binary path");
        let output = std::process::Command::new(exe)
            .args([
                "commands::terminal::tests::terminal_server_info_reports_not_ready_before_bind",
                "--exact",
            ])
            .env(CHILD_MARKER, "1")
            .output()
            .expect("spawn the isolated test process");
        assert!(
            output.status.success(),
            "the isolated process must pass:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Once the listener is bound the command hands the webview the secret, the
    /// port and the capacity it needs, and nothing else.
    #[test]
    fn terminal_server_info_matches_the_bound_server() {
        terminal::server::bind().expect("bind the terminal server");
        let bound = terminal::server::info().expect("a bound server reports its info");
        let reported = terminal_server_info().expect("the command reports the bound server");
        assert_eq!(reported.url, bound.url);
        assert_eq!(reported.port, bound.port);
        assert_eq!(reported.token, bound.token);
        assert_eq!(reported.max_sessions, bound.max_sessions);
        assert_eq!(reported.url, format!("http://127.0.0.1:{}", reported.port));
        assert!(!reported.token.is_empty(), "the secret must be non-empty");
    }
}

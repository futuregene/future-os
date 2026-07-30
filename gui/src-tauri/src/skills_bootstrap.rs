//! Built-in skill bootstrap.
//!
//! `run_builtin_skills` installs the platform's built-in skills by shelling out
//! to the bundled `future` CLI sidecar (`future init`). The CLI is idempotent
//! (skips already-installed skills) and needs no login (the catalogue/download
//! endpoints are unauthenticated). Used by the post-login onboarding flow; runs
//! on a background thread since it blocks on the CLI child process.

use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

const INIT_ARGS: [&str; 1] = ["init"];

/// Force-run the skill bootstrap. Idempotent — the CLI itself skips
/// already-installed skills. Used by the post-login onboarding flow.
pub fn run_builtin_skills(app: &AppHandle) {
    let command = match app.shell().sidecar("future") {
        Ok(command) => command.args(INIT_ARGS),
        Err(error) => {
            eprintln!(
                "FutureOS: bundled CLI sidecar unavailable ({error}); skipping skill bootstrap"
            );
            return;
        }
    };

    let (mut rx, _child) = match command.spawn() {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("FutureOS: failed to start skill bootstrap: {error}");
            return;
        }
    };

    // Drain output to logs and wait for exit.
    let mut exit_code: Option<i32> = None;
    while let Some(event) = rx.blocking_recv() {
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                eprint!("[skills] {}", String::from_utf8_lossy(&bytes));
            }
            CommandEvent::Error(error) => {
                eprintln!("FutureOS: skill bootstrap error: {error}");
            }
            CommandEvent::Terminated(payload) => {
                exit_code = payload.code;
            }
            _ => {}
        }
    }

    if exit_code != Some(0) {
        eprintln!("FutureOS: skill bootstrap did not complete (exit {exit_code:?})");
    }
}

#[cfg(test)]
mod tests {
    use super::INIT_ARGS;

    #[test]
    fn bootstrap_runs_future_init() {
        assert_eq!(INIT_ARGS, ["init"]);
    }
}

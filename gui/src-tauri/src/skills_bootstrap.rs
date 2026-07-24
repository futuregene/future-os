//! First-launch built-in skill bootstrap.
//!
//! On the very first GUI launch (tracked by a one-shot marker in the app
//! settings store, *not* by the presence of `~/.future` — that directory is
//! shared with the TUI/CLI/agent and may already exist), silently install the
//! platform's built-in skills and initialize local commands by shelling out to
//! the bundled `future` CLI sidecar (`future init`). The CLI is idempotent
//! (skips already-installed skills) and needs no login (the
//! catalogue/download endpoints are unauthenticated).
//!
//! Fully silent: all output goes to logs, every failure is swallowed, no window
//! is shown. The marker is set only after a successful run, so a first launch
//! that's offline simply retries on the next launch.

use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

const INIT_ARGS: [&str; 1] = ["init"];

/// Ensure built-in skills are installed once. Safe to call off the launch path;
/// blocks on the CLI child, so run it on a background thread. No-op when already
/// bootstrapped, and in dev (no bundled `future` sidecar) it logs and skips.
///
/// Must run *after* [`crate::future_platform::apply_channel_environment_default`]
/// so the CLI resolves the same platform URL the GUI pinned into `auth.json`.
pub fn ensure_builtin_skills(app: &AppHandle) {
    match crate::store::is_builtin_skills_bootstrapped() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("FutureOS: skill bootstrap check failed: {error}");
            return;
        }
    }

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

    // Drain output to logs and wait for exit. Exit code 0 means the CLI reached
    // the catalogue, attempted installs, and completed platform initialization
    // (per-skill failures don't fail the process) — only then do we consider
    // the bootstrap done.
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

    if exit_code == Some(0) {
        if let Err(error) = crate::store::mark_builtin_skills_bootstrapped() {
            eprintln!("FutureOS: failed to record skill bootstrap: {error}");
        }
    } else {
        eprintln!("FutureOS: skill bootstrap did not complete (exit {exit_code:?}); will retry next launch");
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

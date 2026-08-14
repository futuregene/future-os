//! Built-in skill bootstrap.
//!
//! `run_builtin_skills` installs the platform's built-in skills by shelling out
//! to the bundled `future` CLI sidecar (`future init`). The CLI is idempotent
//! (skips already-installed skills) and needs no login (the catalogue/download
//! endpoints are unauthenticated). Used by the post-login onboarding flow; runs
//! on a background thread since it blocks on the CLI child process.

use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

const INIT_ARGS: [&str; 1] = ["init"];

/// Force-run the skill bootstrap. Idempotent — the CLI itself skips
/// already-installed skills. Used by the post-login onboarding flow.
pub fn run_builtin_skills<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let command = match app.shell().sidecar("future") {
        Ok(command) => command.args(INIT_ARGS),
        Err(error) => {
            eprintln!(
                "FutureOS: bundled CLI sidecar unavailable ({error}); skipping skill bootstrap"
            );
            return;
        }
    };

    let (rx, _child) = match command.spawn() {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("FutureOS: failed to start skill bootstrap: {error}");
            return;
        }
    };

    drain_skill_events(rx);
}

/// Drain the sidecar event channel to the logs, wait for exit, and report a
/// non-zero exit. Extracted so the drain loop + exit check are testable without
/// a real `AppHandle`/sidecar child.
fn drain_skill_events(mut rx: tokio::sync::mpsc::Receiver<CommandEvent>) -> Option<i32> {
    let mut exit_code: Option<i32> = None;
    while let Some(event) = rx.blocking_recv() {
        handle_skill_event(event, &mut exit_code);
    }

    if exit_code != Some(0) {
        eprintln!("FutureOS: skill bootstrap did not complete (exit {exit_code:?})");
    }
    exit_code
}

/// Route a single sidecar event to the logs, updating the exit code on
/// termination. Extracted so the match arms are testable without a real
/// `AppHandle`/sidecar child.
fn handle_skill_event(event: CommandEvent, exit_code: &mut Option<i32>) {
    match event {
        CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
            eprint!("[skills] {}", String::from_utf8_lossy(&bytes));
        }
        CommandEvent::Error(error) => {
            eprintln!("FutureOS: skill bootstrap error: {error}");
        }
        CommandEvent::Terminated(payload) => {
            *exit_code = payload.code;
        }
        // `CommandEvent` is #[non_exhaustive]: the wildcard is required by the
        // compiler even though the released enum has no further variants.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_shell::process::TerminatedPayload;

    #[test]
    fn bootstrap_runs_future_init() {
        assert_eq!(INIT_ARGS, ["init"]);
    }

    #[test]
    fn handle_skill_event_routes_and_captures_exit_code() {
        let mut exit_code = None;
        handle_skill_event(CommandEvent::Stdout(b"hello\n".to_vec()), &mut exit_code);
        handle_skill_event(CommandEvent::Stderr(b"warn\n".to_vec()), &mut exit_code);
        handle_skill_event(CommandEvent::Error("boom".to_string()), &mut exit_code);
        assert_eq!(exit_code, None);
        handle_skill_event(
            CommandEvent::Terminated(TerminatedPayload {
                code: Some(2),
                signal: None,
            }),
            &mut exit_code,
        );
        assert_eq!(exit_code, Some(2));
    }

    #[test]
    fn drain_skill_events_reports_exit_code() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.blocking_send(CommandEvent::Terminated(TerminatedPayload {
            code: Some(0),
            signal: None,
        }))
        .unwrap();
        drop(tx);
        assert_eq!(drain_skill_events(rx), Some(0));
    }

    #[test]
    fn drain_skill_events_reports_nonzero_exit() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.blocking_send(CommandEvent::Terminated(TerminatedPayload {
            code: Some(2),
            signal: None,
        }))
        .unwrap();
        drop(tx);
        assert_eq!(drain_skill_events(rx), Some(2));
    }

    #[test]
    fn run_builtin_skills_logs_spawn_failure() {
        // No bundled `future` sidecar binary in a mock app → the spawn fails and
        // is logged, without draining anything.
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        run_builtin_skills(app.handle());
    }
}

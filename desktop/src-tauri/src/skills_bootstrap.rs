//! Built-in skill bootstrap.
//!
//! `run_builtin_skills` installs the platform's built-in skills by shelling out
//! to the bundled `future` CLI sidecar (`future init`). The CLI is idempotent
//! (skips already-installed skills) and needs no login (the catalogue/download
//! endpoints are unauthenticated). Used by the post-login onboarding flow; runs
//! on a background thread since it blocks on the CLI child process.

use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

const INIT_ARGS: [&str; 1] = ["init"];

/// Force-run the skill bootstrap. Idempotent — the CLI itself skips
/// already-installed skills. Used by the post-login onboarding flow.
pub fn run_builtin_skills<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let executable = match crate::cli_tool::resolve_for("skills.init") {
        Ok(path) => path,
        Err(error) => {
            crate::host_environment::report(&format!("{error}; skipping built-in skill bootstrap"));
            return;
        }
    };
    let command = app.shell().command(executable).args(INIT_ARGS);

    let (rx, child) = match command.spawn() {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("FutureOS: failed to start skill bootstrap: {error}");
            return;
        }
    };

    drain_skill_events(rx, Some(child));
}

/// Drain the sidecar event channel to the logs, wait for exit, and report a
/// non-zero exit. Extracted so the drain loop + exit check are testable without
/// a real `AppHandle`/sidecar child.
fn drain_skill_events(
    mut rx: tokio::sync::mpsc::Receiver<CommandEvent>,
    mut child: Option<CommandChild>,
) -> Option<i32> {
    let mut exit_code: Option<i32> = None;
    let mut stopping = None;
    let mut terminated = false;
    loop {
        if child.is_some() && crate::desktop_shutdown::is_stopping() {
            if let Err(error) = child.take().unwrap().kill() {
                crate::runtime::record_cleanup_error(format!(
                    "CLI bootstrap cancellation failed: {error}"
                ));
            }
            stopping = Some(std::time::Instant::now());
        }
        match rx.try_recv() {
            Ok(event) => {
                terminated = matches!(event, CommandEvent::Terminated(_));
                handle_skill_event(event, &mut exit_code);
                if terminated {
                    break;
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if stopping
                    .is_some_and(|since| since.elapsed() >= std::time::Duration::from_secs(2))
                {
                    crate::runtime::record_cleanup_error(
                        "CLI bootstrap exit was not confirmed".into(),
                    );
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }

    if stopping.is_some() && !terminated {
        crate::runtime::record_cleanup_error("CLI bootstrap termination was not confirmed".into());
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
        assert_eq!(drain_skill_events(rx, None), Some(0));
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
        assert_eq!(drain_skill_events(rx, None), Some(2));
    }

    #[test]
    fn run_builtin_skills_logs_spawn_failure() {
        let _home = crate::auth_store::test_support::HomeGuard::new("bootstrap-no-cli");
        let previous = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        // No adjacent or PATH CLI: production tool discovery fails before any
        // child is spawned, without draining an event channel.
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        run_builtin_skills(app.handle());
        match previous {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
}

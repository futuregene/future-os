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

    /// A sidecar that closes its event channel without ever reporting a
    /// termination ends the drain with the (absent) exit code it saw instead of
    /// spinning on an empty channel.
    #[test]
    fn drain_skill_events_ends_when_the_channel_closes_without_termination() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        assert_eq!(drain_skill_events(rx, None), None);
    }

    /// The CLI fixture used by the spawn tests: answers the `version --json`
    /// probe with a compatible capability contract, and records the fact that
    /// `init` ran — the bootstrap never returns anything, so the marker is the
    /// only honest evidence that the CLI was really executed.
    #[cfg(unix)]
    fn cli_fixture(directory: &std::path::Path, marker: &std::path::Path, init: &str) {
        use std::os::unix::fs::PermissionsExt;
        let capabilities = future_rpc::cli_capabilities::CliCapabilities {
            commands: vec!["skills.init".into()],
            ..Default::default()
        };
        let reply = serde_json::json!({ "version": "1.2.3", "desktopCapabilities": capabilities })
            .to_string();
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("future");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = version ]; then printf '%s' '{reply}'; exit 0; fi\n\
                 printf 'ran' > '{marker}'\n{init}\n",
                marker = marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app")
    }

    /// The onboarding step really spawns the CLI through the shell plugin: the
    /// marker is the proof that `future init` ran as a child of this app.
    #[cfg(unix)]
    #[test]
    fn the_bootstrap_spawns_the_cli_through_the_shell_plugin() {
        let _home = crate::auth_store::test_support::HomeGuard::new("bootstrap-spawn");
        let root = std::env::temp_dir().join(format!("futureos-bootstrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let marker = root.join("init-ran");
        let directory = root.join("bin");
        cli_fixture(&directory, &marker, "printf 'installed 2 skills\\n'");
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", directory.as_os_str());

        let app = mock_app();
        run_builtin_skills(app.handle());

        match saved {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        assert!(
            marker.exists(),
            "the spawned CLI must have run its `init` (drain loop exit {:?})",
            marker
        );
    }

    /// A CLI that cannot be reached by the spawn is reported and the drain is
    /// skipped entirely — the onboarding step never blocks or panics when the
    /// sidecar is unusable.
    #[cfg(unix)]
    #[test]
    fn a_cli_outside_the_runnable_contract_is_reported_and_ignored() {
        let _home = crate::auth_store::test_support::HomeGuard::new("bootstrap-unusable");
        let root = std::env::temp_dir().join(format!("futureos-unusable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        // A *directory* named `future`: every candidate is judged unusable, so
        // discovery fails before any child is spawned.
        std::fs::create_dir_all(root.join("bin").join("future")).unwrap();
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", root.join("bin").as_os_str());

        let app = mock_app();
        run_builtin_skills(app.handle());

        match saved {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
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

#[cfg(all(test, unix))]
mod block_fixtures {
    //! The `skills_bootstrap` stop and spawn-failure arms, driven through a
    //! subprocess re-exec: `desktop_shutdown::begin` closes producer admission for
    //! the whole process and nothing clears it (§6), so closing it in the shared
    //! libtest process would change every later test's drain. Each case gets its
    //! own `HOME`, `FUTURE_HOME` and agent socket; the child is spawned by the
    //! **real** shell plugin, not a mock, because `drain_skill_events` takes a
    //! `CommandChild` no mock can produce.
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const CASE: &str = "FUTURE_SKILLS_BOOTSTRAP_CASE";

    /// Discovery succeeds and the CLI is still byte-identical, but the program it
    /// names as its interpreter is gone: the spawn fails, the drain is skipped,
    /// and `init` never runs.
    #[test]
    fn a_cli_whose_runtime_vanished_after_discovery_is_reported_and_skipped() {
        run_case("spawn-failure");
    }

    /// A stop that arrives with a live child kills it and ends the drain at once,
    /// instead of waiting out the unconfirmed-exit budget.
    #[test]
    fn a_stop_kills_a_live_bootstrap_child_and_ends_the_drain() {
        run_case("stop-live");
    }

    /// The drain's own budget: when the child's exit cannot be observed (a
    /// surviving grandchild holds the event pipes open) the loop ends after its
    /// wait instead of blocking shutdown, and reports the exit as unconfirmed.
    #[test]
    fn a_surviving_grandchild_ends_the_drain_at_its_budget() {
        run_case("stop-unconfirmed");
    }

    fn run_case(case: &str) {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
        std::fs::create_dir_all(&parent).unwrap();
        let home = tempfile::tempdir_in(&parent).unwrap();
        // A unix socket path is at most 104 bytes on macOS, so it lives in the
        // short system temp dir rather than beside the fixture home.
        let socket = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "skills_bootstrap::block_fixtures::host_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CASE, case)
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("FUTURE_HOME", home.path().join(".future"))
            .env("FUTURE_AGENT_SOCKET", socket.path().join("agent.sock"))
            .env_remove("FUTURE_AGENT_GRPC_ADDR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut finished = false;
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                finished = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !finished {
            let _ = child.kill();
        }
        let output = child.wait_with_output().unwrap();
        // Hoisted: a passing `assert!` never evaluates its message arguments.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            finished && output.status.success(),
            "{case}: {stdout} {stderr}"
        );
    }

    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app")
    }

    fn scoped(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("futureos-sk-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Polls a condition for a bounded time. The trailing call is the timeout
    /// answer: reaching it means the assertion after it fails, the same
    /// failure-only class as `pty.rs`'s `wait_for` helpers.
    fn wait_until(ready: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if ready() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        ready()
    }

    fn wait_for(path: &Path) -> bool {
        wait_until(|| path.exists())
    }

    fn alive(pid: u32) -> bool {
        (unsafe { libc::kill(pid as i32, 0) }) == 0
    }

    fn version_reply() -> String {
        let capabilities = future_rpc::cli_capabilities::CliCapabilities {
            commands: vec!["skills.init".into()],
            ..Default::default()
        };
        serde_json::json!({ "version": "1.2.3", "desktopCapabilities": capabilities }).to_string()
    }

    #[test]
    fn host_fixture() {
        let Ok(case) = std::env::var(CASE) else {
            return;
        };
        if case == "spawn-failure" {
            let root = scoped("spawn-failure");
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let interpreter = root.join("sh");
            std::os::unix::fs::symlink("/bin/sh", &interpreter).unwrap();
            let marker = root.join("init-ran");
            // The CLI answers the probe through an interpreter symlink. Removing
            // that link leaves the CLI's own identity — size, mtime, inode, ctime —
            // untouched, so the post-probe re-read still accepts it; only the
            // spawn can notice.
            script(
                &bin.join("future"),
                &format!(
                    "#!{}\nif [ \"$1\" = version ]; then printf '%s' '{}'; exit 0; fi\nprintf 'ran' > '{}'\n",
                    interpreter.display(),
                    version_reply(),
                    marker.display()
                ),
            );
            std::env::set_var("PATH", &bin);
            let discovered = crate::cli_tool::resolve_for("skills.init");
            assert!(discovered.is_ok(), "discovery must succeed: {discovered:?}");
            // The step that happens between discovery and spawn in the field: the
            // CLI is untouched, the program it needs is gone.
            std::fs::remove_file(&interpreter).unwrap();
            let app = mock_app();
            run_builtin_skills(app.handle());
            assert!(
                !marker.exists(),
                "a CLI that cannot be spawned must never run `init`"
            );
            return;
        }
        if case == "stop-live" {
            let root = scoped("stop-live");
            let marker = root.join("up");
            let bootstrap = root.join("bootstrap.sh");
            // `exec` makes the sleep the direct child, so the kill reaches it.
            script(
                &bootstrap,
                &format!(
                    "#!/bin/sh\nprintf 'up' > '{}'\nexec sleep 300\n",
                    marker.display()
                ),
            );
            let app = mock_app();
            let (rx, child) = app.shell().command(&bootstrap).spawn().unwrap();
            let pid = child.pid();
            assert!(wait_for(&marker), "the fixture child must start");
            crate::desktop_shutdown::begin("fixture");
            let started = Instant::now();
            let code = drain_skill_events(rx, Some(child));
            let elapsed = started.elapsed();
            assert!(
                elapsed < Duration::from_secs(2),
                "a killed child is reported, not waited out: {elapsed:?}"
            );
            assert_eq!(code, None, "a signalled child reports no exit code");
            assert!(!alive(pid), "the stop must kill the bootstrap child");
            return;
        }
        if case == "stop-unconfirmed" {
            let root = scoped("stop-unconfirmed");
            let marker = root.join("up");
            let grandchild = root.join("grandchild");
            let bootstrap = root.join("bootstrap.sh");
            // The grandchild inherits the plugin's stdout/stderr pipes and keeps
            // them open, so no `Terminated` event can ever be delivered even after
            // the direct child exits: the drain's only exit is its own budget.
            script(
                &bootstrap,
                &format!(
                    "#!/bin/sh\nsleep 30 &\nprintf '%s' \"$!\" > '{}'\nprintf 'up' > '{}'\nexit 0\n",
                    grandchild.display(),
                    marker.display()
                ),
            );
            let app = mock_app();
            let (rx, child) = app.shell().command(&bootstrap).spawn().unwrap();
            let pid = child.pid();
            assert!(
                wait_for(&marker) && wait_for(&grandchild),
                "the fixture must start"
            );
            // A zombie still answers signal 0; ESRCH is the plugin's wait thread
            // having reaped it. The drain then finds no exit to report, which is
            // the state this case is about.
            assert!(
                wait_until(|| !alive(pid)),
                "the direct child must be reaped before the stop"
            );
            let survivor: i32 = std::fs::read_to_string(&grandchild)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            crate::desktop_shutdown::begin("fixture");
            let started = Instant::now();
            let code = drain_skill_events(rx, Some(child));
            let elapsed = started.elapsed();
            unsafe { libc::kill(survivor, libc::SIGKILL) };
            assert!(
                elapsed >= Duration::from_secs(2),
                "an unobservable exit is waited out, not guessed: {elapsed:?}"
            );
            assert_eq!(code, None, "no exit code was ever delivered");
            return;
        }
        panic!("unknown case {case}");
    }
}

// Explorer must start the GUI without first allocating a console window.
#![cfg_attr(
    all(windows, feature = "gui", not(test)),
    windows_subsystem = "windows"
)]

#[cfg(not(test))]
use futureos_lib::launch::{self, Launch};

#[cfg(not(test))]
fn main() -> std::process::ExitCode {
    // Reuse a launching shell for help and error output (a terminal launch of
    // `futureos --help` or a refused flag). Explorer has no console to attach to;
    // never allocate one for a GUI launch.
    #[cfg(all(windows, feature = "gui"))]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(result) = futureos_lib::dispatch_agent_helper(&args) {
        return match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    match Launch::parse(&args) {
        Ok(Launch::Help) => {
            println!("{}", launch::HELP);
            std::process::ExitCode::SUCCESS
        }
        Ok(Launch::Gui) => start_desktop(),
        Err(error) => {
            eprintln!("{error}\n\n{}", launch::HELP);
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(test))]
fn start_desktop() -> std::process::ExitCode {
    #[cfg(feature = "gui")]
    {
        configure_environment();
        match futureos_lib::run() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("FutureOS: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(feature = "gui"))]
    {
        eprintln!("This build has no graphical interface. Run future host instead.");
        std::process::ExitCode::FAILURE
    }
}

/// Suppress macOS activity logs for graphical launches. Windows console
/// detachment happens only after the GUI has acquired its instance lock.
#[cfg(any(feature = "gui", test))]
fn configure_environment() {
    #[cfg(target_os = "macos")]
    std::env::set_var("OS_ACTIVITY_MODE", "disable");
}

#[cfg(test)]
mod tests {
    /// Suppressing the macOS activity log is an *effect*, and the function is
    /// compiled into the test build on purpose (`cfg(any(feature = "gui", test))`),
    /// so assert the effect instead of only taking its address: a launch that
    /// stopped applying it would otherwise stay green.
    #[test]
    fn configure_environment_suppresses_macos_activity_logs() {
        std::env::remove_var("OS_ACTIVITY_MODE");
        super::configure_environment();
        #[cfg(target_os = "macos")]
        assert_eq!(std::env::var("OS_ACTIVITY_MODE").as_deref(), Ok("disable"));
        #[cfg(not(target_os = "macos"))]
        assert!(std::env::var("OS_ACTIVITY_MODE").is_err());
    }

    /// The sandbox helper is dispatched from `main` before any Desktop or
    /// headless state is built, and a refused request must be *reported* rather
    /// than swallowed — the exit code and the stderr line are the contract the
    /// sandbox host checks. The flag is Linux-only, so on Linux the same argv
    /// would run the real helper (agent/src/cli.rs:213-218); the refusal asserted
    /// here is every other platform's behaviour, which is why this test is gated.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_sandbox_helper_request_is_dispatched_before_any_startup() {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(launcher_binary("futureos"))
            .arg("--linux-sandbox-helper")
            .arg("probe")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("FUTURE_HOME", home.path().join(".future"))
            .env_remove("FUTURE_AGENT_SOCKET")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run the graphical entry point");

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains("Linux sandbox helper is unavailable on this platform"),
            "{stderr}"
        );
        assert!(output.stdout.is_empty(), "{stderr}");
        // Dispatch happens before the data directory is resolved or created.
        assert!(!home.path().join(".future").exists());
    }

    /// The binary cargo built next to this test harness. The integration tests
    /// reach the same artifact through `CARGO_BIN_EXE_*`, a variable cargo only
    /// defines for integration targets; a unit test inside the bin gets the path
    /// from its own location instead (`<target>/<profile>/deps/<test-bin>` →
    /// `<target>/<profile>/<name>`), which is the layout `cargo test` uses.
    ///
    /// Requires the package's normal test build: `cargo test` builds the bins
    /// because `tests/desktop_entry.rs` exists, but `cargo test --bin futureos`
    /// alone does not.
    #[cfg(not(target_os = "linux"))]
    fn launcher_binary(name: &str) -> std::path::PathBuf {
        let path = std::env::current_exe()
            .expect("the test harness path")
            .parent()
            .and_then(|deps| deps.parent())
            .expect("the cargo profile directory")
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let expected = path.display().to_string();
        assert!(
            path.is_file(),
            "expected the built launcher at {expected} (run `cargo test` for the package)"
        );
        path
    }
}

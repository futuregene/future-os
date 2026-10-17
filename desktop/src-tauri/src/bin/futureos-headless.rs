//! Standalone server entrypoint; never ship this binary linked to GUI libraries.
#[cfg(feature = "gui")]
compile_error!("futureos-headless requires --no-default-features --features headless");

use futureos_lib::headless::{Launch, Options, HELP};

fn main() -> std::process::ExitCode {
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
    let result = match Options::parse(&args) {
        Ok(Launch::Help) => {
            println!("{HELP}");
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Launch::Headless(options)) => futureos_lib::headless::run(options),
        Err(error) => Err(format!("{error}\n\n{HELP}")),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("FutureOS: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    /// The standalone entry point dispatches the sandbox helper before it parses
    /// its own options, so the helper's refusal is what a sandbox host sees — the
    /// exit code (1) and the message on stderr, not a started Desktop. The flag is
    /// Linux-only (agent/src/cli.rs:213-218): on Linux this argv would run the real
    /// helper, which is why the test is gated to the platforms that refuse it.
    ///
    /// Spawning is the only way in: under `cargo test` libtest owns `main`, so the
    /// binary's own body never runs in-process.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_sandbox_helper_request_is_dispatched_before_any_startup() {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(launcher_binary())
            .arg("--linux-sandbox-helper")
            .arg("probe")
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("FUTURE_HOME", home.path().join(".future"))
            .env_remove("FUTURE_AGENT_SOCKET")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run the standalone entry point");

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains("Linux sandbox helper is unavailable on this platform"),
            "{stderr}"
        );
        assert!(output.stdout.is_empty(), "{stderr}");
        assert!(!home.path().join(".future").exists());
    }

    /// The binary cargo built next to this test harness (see
    /// `tests/headless_cli.rs`, which uses the integration-target spelling
    /// `CARGO_BIN_EXE_*`). Requires the package's normal test build, which `cargo
    /// test` performs for every bin because an integration test target exists.
    #[cfg(not(target_os = "linux"))]
    fn launcher_binary() -> std::path::PathBuf {
        let path = std::env::current_exe()
            .expect("the test harness path")
            .parent()
            .and_then(|deps| deps.parent())
            .expect("the cargo profile directory")
            .join(format!("futureos-headless{}", std::env::consts::EXE_SUFFIX));
        let expected = path.display().to_string();
        assert!(
            path.is_file(),
            "expected the built launcher at {expected} (run `cargo test` for the package)"
        );
        path
    }
}

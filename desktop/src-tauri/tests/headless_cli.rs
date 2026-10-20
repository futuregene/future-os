//! Retained Desktop launch/retirement wiring. Actual headless process checks live in CLI.
#![cfg(feature = "gui")]

use std::path::Path;
use std::process::{Command, Stdio};

fn desktop(home: &Path) -> Command {
    isolated_command(env!("CARGO_BIN_EXE_futureos"), home)
}

fn isolated_command(binary: &str, home: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("FUTURE_HOME", home.join(".future"))
        .env_remove("FUTURE_AGENT_SOCKET")
        .env("FUTURE_AGENT_GRPC_ADDR", "http://[::1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[test]
fn help_and_invalid_arguments_have_no_startup_side_effects() {
    let home = tempfile::tempdir().unwrap();
    let help = desktop(home.path()).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: futureos [--relay"));
    assert!(!home.path().join(".future").exists());
    for args in [
        vec!["--headless"],
        vec!["--headless", "--relay", "https://relay.example"],
        vec!["--help", "--headless"],
    ] {
        let invalid = desktop(home.path()).args(args).output().unwrap();
        assert_eq!(invalid.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("future headless"));
        assert!(invalid.stdout.is_empty());
        assert!(!home.path().join(".future").exists());
    }
    // The pairing flags this entry point used to take are refused by name, and a
    // refused launch still touches no state.
    for flag in ["--re-pair", "--no-qr"] {
        let invalid = desktop(home.path()).arg(flag).output().unwrap();
        assert_eq!(invalid.status.code(), Some(1), "{flag}");
        let stderr = String::from_utf8_lossy(&invalid.stderr);
        assert!(stderr.contains(flag), "{flag}: {stderr}");
        assert!(invalid.stdout.is_empty(), "{flag}");
        assert!(!home.path().join(".future").exists(), "{flag}");
    }
}

#[cfg(not(feature = "gui"))]
#[test]
fn gui_free_build_points_to_standalone_entrypoint() {
    let home = tempfile::tempdir().unwrap();
    let result = desktop(home.path()).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("future headless"));
    assert!(!home.path().join(".future").exists());
}

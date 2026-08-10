//! CLI smoke tests: run the real `future-tui` binary end-to-end.
//!
//! These cover `main.rs` (the binary entry point) and the top-level
//! `index::run` flow, which unit tests cannot reach.

use std::process::Command;

/// Run the binary with a scratch HOME so no user config is touched.
fn run_with_args(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().expect("tempdir");
    Command::new(env!("CARGO_BIN_EXE_future-tui"))
        .args(args)
        .env("HOME", home.path())
        .output()
        .expect("spawn future-tui")
}

#[test]
fn help_flag_prints_usage_and_exits_zero() {
    let out = run_with_args(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("future-tui"));
    assert!(stdout.contains("--help"));
}

#[test]
fn version_flag_prints_version_and_exits_zero() {
    let out = run_with_args(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("future-tui v"));
}

#[test]
fn unknown_option_exits_one() {
    let out = run_with_args(&["--definitely-not-an-option"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Unknown option"));
}

#[test]
fn print_mode_without_message_exits_one() {
    let out = run_with_args(&["--print"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn list_models_without_agent_exits_one() {
    // Nothing listens on this addr — the connect must fail fast.
    let out = run_with_args(&["--list-models", "--grpc-addr", "127.0.0.1:1"]);
    assert_eq!(out.status.code(), Some(1));
}

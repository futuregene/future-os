//! Integration tests for the `future` binary itself (main() + embedded
//! component dispatch). These spawn the real executable as a subprocess,
//! which also lets cargo-llvm-cov capture main()'s coverage through the
//! inherited LLVM_PROFILE_FILE.

use std::process::Command;

fn future(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_future"))
        .args(args)
        .output()
        .expect("spawn future binary");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn version_flag() {
    let (code, stdout, stderr) = future(&["--version"]);
    assert_eq!(code, Some(0));
    assert!(stdout.starts_with("future v"), "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn no_args_prints_main_help() {
    let (code, stdout, _) = future(&[]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("future"), "stdout: {stdout}");
}

#[test]
fn embedded_agent_help() {
    // clap prints help and exits 0 inside the embedded agent entry.
    let (code, stdout, _) = future(&["agent", "--help"]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("future-agent"), "stdout: {stdout}");
}

#[test]
fn embedded_agent_rejects_unknown_flag() {
    // clap rejects the flag before the server starts → exit 1 through
    // run_agent's error arm.
    let (code, _, stderr) = future(&["agent", "--bogus-flag-xyz"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("Error"), "stderr: {stderr}");
}

#[test]
fn embedded_tui_version_and_unknown_option() {
    let (code, stdout, _) = future(&["tui", "--version"]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("future-tui v"), "stdout: {stdout}");

    let (code, _, stderr) = future(&["tui", "--bogus-option"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("Unknown option"), "stderr: {stderr}");
}

#[test]
fn embedded_channel_version() {
    let (code, stdout, _) = future(&["channel", "--version"]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("future-channel v"), "stdout: {stdout}");
    // "channels" is an alias.
    let (code, _, _) = future(&["channels", "--version"]);
    assert_eq!(code, Some(0));
}

#[test]
fn embedded_loop_unknown_command() {
    let (code, _, stderr) = future(&["loop", "bogus-cmd-xyz"]);
    assert_eq!(code, Some(1));
    assert!(!stderr.is_empty());
}

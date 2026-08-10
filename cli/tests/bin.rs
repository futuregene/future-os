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
fn embedded_agent_run_failure_is_exit_1() {
    // Port 1 fails to serve → run_agent's error arm (agent logs go to
    // stdout via tracing).
    let (code, stdout, _) = future(&["agent", "--grpc-addr", "127.0.0.1:1"]);
    assert_eq!(code, Some(1));
    assert!(stdout.contains("exited with error"), "stdout: {stdout}");
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

#[test]
fn tools_call_reads_args_from_stdin() {
    use std::io::Write;
    use std::process::Stdio;
    // `tools call web_search --stdin` reads the JSON args from stdin; the
    // call then fails without an API key, but the stdin path executed.
    let mut child = Command::new(env!("CARGO_BIN_EXE_future"))
        .args(["tools", "call", "web_search", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn future");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(br#"{"query":"x"}"#)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn embedded_channel_invalid_config_errors() {
    // An existing-but-invalid channels config makes run() return Err → the
    // main.rs error arm (exit 1) instead of starting the bridge.
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".future").join("channels");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir");
    std::fs::write(cfg_dir.join("config.json"), "{not json").expect("write");
    let output = Command::new(env!("CARGO_BIN_EXE_future"))
        .args(["channel"])
        .env("HOME", dir.path())
        .env("FUTURE_HOME", dir.path())
        .output()
        .expect("run future channel");
    assert_eq!(output.status.code(), Some(1));
}

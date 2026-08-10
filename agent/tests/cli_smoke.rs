//! End-to-end smoke tests for the `future-agent` binary entry point
//! (main.rs + cli.rs). The full startup path mutates process-global state
//! (tracing subscriber, login-shell env hydration), so it runs as a real
//! subprocess; `--profile-seconds 0` makes the agent shut itself down right
//! after the gRPC server is up.

use std::process::Command;

fn isolated_home() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp home")
}

#[test]
fn agent_starts_serves_and_shuts_down_via_profile_timer() {
    let home = isolated_home();
    let profile = home.path().join("flame.svg");
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args([
            "--grpc-addr",
            "127.0.0.1:0",
            "--profile-seconds",
            "0",
            "--profile",
            profile.to_str().unwrap(),
            "--verbose",
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "agent exited with {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The flamegraph was written on shutdown.
    assert!(
        profile.exists(),
        "profile output missing; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn agent_prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .arg("--version")
        .output()
        .expect("spawn future-agent");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("future-agent"));
}

#[test]
fn agent_rejects_unknown_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .arg("--definitely-not-a-flag")
        .output()
        .expect("spawn future-agent");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected argument"));
}

//! Subprocess tests for the `future-channel` binary: main() + lib.rs run()
//! paths that are process-global (crypto provider, tracing, ctrl-c) or need
//! real signals.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_future-channel"))
}

fn isolated_home(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("future-channel-bin-tests")
        .join(format!("{}-{}", label, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_config(home: &std::path::Path, contents: &str) {
    let dir = home.join(".future").join("channels");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), contents).unwrap();
}

#[test]
fn version_flag() {
    let out = bin().arg("--version").output().expect("run binary");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("future-channel v"), "{stdout}");
}

#[test]
fn missing_config_writes_defaults_and_exits_nonzero() {
    // First run writes the default config and exits non-zero ("edit it and
    // restart") — the file now exists, so run_async returns the load error.
    let home = isolated_home("missing-config");
    let out = bin().env("HOME", &home).output().expect("run binary");
    assert!(!out.status.success());
    assert!(home.join(".future/channels/config.json").exists());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Default config written"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn unwritable_home_warns_and_exits_ok() {
    // The default config can't be written (HOME is read-only) → load fails
    // without creating the file → warn + Ok (graceful degradation).
    let home = isolated_home("readonly-home");
    let mut perms = std::fs::metadata(&home).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o555);
    }
    std::fs::set_permissions(&home, perms).unwrap();
    let out = bin().env("HOME", &home).output().expect("run binary");
    // Best-effort permission restore so cleanup works.
    let mut perms = std::fs::metadata(&home).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    let _ = std::fs::set_permissions(&home, perms);
    assert!(
        out.status.success(),
        "status: {:?}, stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn invalid_config_exits_nonzero() {
    let home = isolated_home("invalid-config");
    write_config(&home, "not json {{{");
    let out = bin().env("HOME", &home).output().expect("run binary");
    assert!(!out.status.success());
}

#[test]
fn dingtalk_enabled_without_credentials_exits_nonzero() {
    let home = isolated_home("dt-no-creds");
    write_config(&home, r#"{"dingtalk": {"enabled": true}}"#);
    let out = bin().env("HOME", &home).output().expect("run binary");
    assert!(!out.status.success());
}

/// Spawn the binary with the given config, wait for startup, send SIGINT,
/// and assert a clean exit(0) (the ctrl-c → shutdown path).
#[cfg(unix)]
fn sigint_shutdown_case(label: &str, config: &str) {
    let home = isolated_home(label);
    write_config(&home, config);
    let mut child = bin()
        .env("HOME", &home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn binary");
    // Give the runtime a moment to reach the ctrl-c wait.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let pid = child.id() as i32;
    unsafe { libc::kill(pid, libc::SIGINT) };
    let status = child.wait().expect("wait");
    assert!(status.success(), "clean shutdown expected, got {status:?}");
}

#[cfg(unix)]
#[test]
fn no_channels_enabled_sigint_exits_cleanly() {
    sigint_shutdown_case("no-channels", "{}");
}

#[cfg(unix)]
#[test]
fn enabled_channel_with_dead_agent_sigint_exits_cleanly() {
    // Feishu enabled with credentials but no reachable agent: the channel
    // task errors out, the main task still shuts down cleanly on SIGINT.
    sigint_shutdown_case(
        "dead-agent",
        r#"{"agent": {"grpc_addr": "http://127.0.0.1:1"},
            "feishu": {"enabled": true, "app_id": "x", "app_secret": "y"}}"#,
    );
}

#[cfg(unix)]
#[test]
fn dingtalk_enabled_with_dead_agent_sigint_exits_cleanly() {
    // Same for the DingTalk spawn arm.
    sigint_shutdown_case(
        "dt-dead-agent",
        r#"{"agent": {"grpc_addr": "http://127.0.0.1:1"},
            "dingtalk": {"enabled": true, "client_id": "x", "client_secret": "y"}}"#,
    );
}

#[cfg(unix)]
#[test]
fn both_channels_enabled_sigint_exits_cleanly() {
    sigint_shutdown_case(
        "both-dead-agent",
        r#"{"agent": {"grpc_addr": "http://127.0.0.1:1"},
            "feishu": {"enabled": true, "app_id": "x", "app_secret": "y"},
            "dingtalk": {"enabled": true, "client_id": "x", "client_secret": "y"}}"#,
    );
}

#[cfg(unix)]
#[test]
fn disabled_channels_sigint_exits_cleanly() {
    // Channels present but disabled → the enabled-check false paths.
    sigint_shutdown_case(
        "disabled-channels",
        r#"{"feishu": {"enabled": false, "app_id": "x", "app_secret": "y"},
            "dingtalk": {"enabled": false, "client_id": "x", "client_secret": "y"}}"#,
    );
}

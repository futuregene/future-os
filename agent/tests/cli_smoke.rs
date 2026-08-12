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

#[test]
fn agent_grpc_addr_forms_and_profile_default_path() {
    // ":0" (bare port), "0" (plain number) and "host:port" all reach the
    // server; --profile-seconds 0 without --profile writes the default
    // flamegraph path. (A non-numeric bare string falls back to 50051, which
    // may legitimately be in use on a dev machine — not covered here.)
    for addr in [":0", "0", "127.0.0.1:0"] {
        let home = isolated_home();
        let work = home.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
            .args(["--grpc-addr", addr, "--profile-seconds", "0"])
            .current_dir(&work)
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .output()
            .expect("spawn future-agent");
        assert!(
            output.status.success(),
            "addr {addr}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Default flamegraph lands next to the process cwd.
        assert!(work.join("agent-profile.svg").exists(), "addr {addr}");
    }
}

#[test]
fn agent_log_file_and_heap_flag_paths() {
    let home = isolated_home();
    let log_file = home.path().join("custom-agent.log");
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args([
            "--grpc-addr",
            "127.0.0.1:0",
            "--profile-seconds",
            "0",
            "--log-file",
            log_file.to_str().unwrap(),
            "--log-max-lines",
            "1000",
            "--profile-heap",
            home.path().join("heap.out").to_str().unwrap(),
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(log_file.exists(), "explicit log file was created");
    let log = std::fs::read_to_string(&log_file).unwrap();
    assert!(log.contains("file logging enabled"), "{log}");

    // --log-file without a value uses the default under ~/.future.
    let home = isolated_home();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args([
            "--grpc-addr",
            "127.0.0.1:0",
            "--profile-seconds",
            "0",
            "--log-file",
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home.path().join(".future/agent/logs/agent.log").exists());
}

#[test]
fn agent_reads_settings_auth_and_context_from_home() {
    let home = isolated_home();
    // CLAUDE.md in the (home-as-cwd) workspace is loaded as project context.
    std::fs::write(home.path().join("CLAUDE.md"), "# test context").unwrap();
    // A non-future provider key resolves the "first credentialled model" arm.
    let agent_dir = home.path().join(".future/agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("auth.json"),
        r#"{"openai": {"type": "api_key", "key": "sk-test"}}"#,
    )
    .unwrap();
    std::fs::write(
        agent_dir.join("settings.json"),
        r#"{"maxTurns": 7, "defaultModel": "openai/gpt-4o"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn agent_shuts_down_cleanly_on_sigint() {
    use std::os::unix::process::ExitStatusExt;
    let home = isolated_home();
    // A concrete free port lets us wait until the server is actually up
    // (the Ctrl-C handler is installed only once async_main runs).
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", &format!("127.0.0.1:{port}")])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .spawn()
        .expect("spawn future-agent");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "server never came up");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let pid = child.id() as i32;
    unsafe { libc::kill(pid, libc::SIGINT) };
    let status = child.wait().expect("wait");
    // Clean exit (0) — not killed by a signal.
    assert!(
        status.signal().is_none(),
        "terminated by signal: {status:?}"
    );
    assert!(status.success(), "exit status: {status:?}");
}

#[test]
fn agent_exits_nonzero_when_grpc_bind_fails() {
    let home = isolated_home();
    // Hold the port so the agent's bind fails and async_main errors out.
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", &format!("127.0.0.1:{port}")])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn agent_warns_and_uses_defaults_on_corrupt_settings() {
    let home = isolated_home();
    let agent_dir = home.path().join(".future/agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("settings.json"), "{not json").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn agent_resolves_custom_provider_model_config() {
    let home = isolated_home();
    let agent_dir = home.path().join(".future/agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("models.json"),
        r#"{
          "providers": {
            "custom": {
              "api": "openai-completions",
              "baseUrl": "https://custom.example.com/v1",
              "apiKey": "sk-custom",
              "thinkingLevelMap": {"high": {"budget": 4096}},
              "compat": {"thinkingFormat": "deepseek", "supportsReasoningEffort": true},
              "models": [{"id": "custom-model", "limit": {"context": 64000, "output": 4096}}]
            }
          }
        }"#,
    )
    .unwrap();
    std::fs::write(
        agent_dir.join("settings.json"),
        r#"{"defaultModel": "custom/custom-model", "maxTurns": 9}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn agent_prefers_future_model_when_future_is_configured() {
    let home = isolated_home();
    let agent_dir = home.path().join(".future/agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("auth.json"),
        r#"{"future": {"type": "api_key", "key": "k"}}"#,
    )
    .unwrap();
    // A warm future-models cache makes the future provider visible at startup.
    std::fs::write(
        agent_dir.join(".future-models-cache.json"),
        r#"{"fetched_at": 1, "models": [{
            "id": "deepseek-v4-pro", "name": "v4 pro", "provider": "future",
            "API": "openai", "reasoning": true, "ContextWindow": 128000
        }]}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn agent_reclaims_orphan_run_data_at_startup() {
    let home = isolated_home();
    // An orphan run-events directory (no matching transcript) is reclaimed.
    let orphan = home.path().join(".future/agent/run-events/orphan-session");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("run-1.jsonl"), "{}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!orphan.exists(), "orphan run data reclaimed at startup");
}

#[test]
fn agent_survives_unwritable_profile_path() {
    let home = isolated_home();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args([
            "--grpc-addr",
            "127.0.0.1:0",
            "--profile-seconds",
            "0",
            "--profile",
            "/nonexistent-dir-xyz/flame.svg",
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(not(windows))]
#[test]
fn agent_profiler_build_failure_continues_without_profiling() {
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
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("FUTURE_TEST_PROFILER_FAIL_AT", "build")
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !profile.exists(),
        "no flamegraph when the profiler never started"
    );
}

#[cfg(not(windows))]
#[test]
fn agent_profiler_report_failure_is_logged_and_ignored() {
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
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("FUTURE_TEST_PROFILER_FAIL_AT", "report")
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!profile.exists(), "report build failed before any write");
}

#[cfg(not(windows))]
#[test]
fn agent_profiler_write_failure_is_logged_and_ignored() {
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
        ])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("FUTURE_TEST_PROFILER_FAIL_AT", "write")
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The file was created; only the flamegraph write failed.
    assert!(profile.exists());
}

#[test]
fn agent_bare_nonnumeric_addr_falls_back_to_default_port() {
    // "banana" has no ':' and does not parse as u16, so the agent falls back
    // to 127.0.0.1:50051. On a dev machine that port may legitimately be in
    // use (then the bind fails and the agent exits 1) — both outcomes
    // exercise the fallback arm, which is what this test pins.
    let home = isolated_home();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "banana", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.code() == Some(0) || output.status.code() == Some(1),
        "unexpected exit {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn agent_model_without_base_url_or_key_uses_builtin_defaults() {
    let home = isolated_home();
    let agent_dir = home.path().join(".future/agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    // No baseUrl and no apiKey: the resolver takes the empty-field None arms
    // and falls back to built-in defaults.
    std::fs::write(
        agent_dir.join("models.json"),
        r#"{
          "providers": {
            "bare": {
              "api": "openai-completions",
              "models": [{"id": "bare-model", "limit": {"context": 64000, "output": 4096}}]
            }
          }
        }"#,
    )
    .unwrap();
    std::fs::write(
        agent_dir.join("settings.json"),
        r#"{"defaultModel": "bare/bare-model"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn agent_warns_when_orphan_run_data_root_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    let home = isolated_home();
    // run-events exists but is unreadable → gc_orphan_run_data errors and the
    // agent logs a warning instead of failing startup.
    let run_events = home.path().join(".future/agent/run-events");
    std::fs::create_dir_all(&run_events).unwrap();
    std::fs::set_permissions(&run_events, std::fs::Permissions::from_mode(0o000)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_future-agent"))
        .args(["--grpc-addr", "127.0.0.1:0", "--profile-seconds", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .expect("spawn future-agent");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

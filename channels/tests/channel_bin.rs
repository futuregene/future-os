//! Subprocess tests for the `future-channel` binary: main() + lib.rs run()
//! paths that are process-global (crypto provider, tracing, ctrl-c) or need
//! real signals.

use std::process::Command;

fn bin() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_future-channel"));
    // Readiness checks observe INFO logs, regardless of the developer's own
    // RUST_LOG setting (for example, `warn` would hide the webhook marker).
    command.env("RUST_LOG", "info");
    command
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

/// Ask a running child to stop with SIGINT, retrying until it exits.
///
/// A single `kill` after a fixed sleep is a race this harness cannot win, in
/// two different ways that both showed up on a loaded machine (load average
/// ~11, these cases all at once):
///
/// * **Before the child's Ctrl-C listener exists**, SIGINT still has its
///   default disposition, so the child dies *from the signal* instead of
///   shutting down (observed: `ExitStatus(2)`). [`sigint_scenario`] retries the
///   whole case with a longer margin for that.
/// * **After** it exists a signal can still be missed, and the child then waits
///   for the next one forever: the test used to hang (it had no timeout at all),
///   and a run could only be unblocked by sending a second SIGINT by hand.
///   Ctrl-C is a level-triggered request to stop, so re-sending it until the
///   child exits is what the product actually promises.
///
/// The deadline turns a wedged child into a failure with a message instead of a
/// hung test binary.
#[cfg(unix)]
fn sigint_until_exit(child: &mut std::process::Child) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let pid = child.id() as i32;
    loop {
        unsafe { libc::kill(pid, libc::SIGINT) };
        // Give the child time to run its shutdown path before asking again.
        let attempt = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < attempt {
            if let Some(status) = child.try_wait().expect("try_wait") {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the child ignored SIGINT for 30s"
        );
    }
}

/// How long to wait before the first SIGINT of an attempt: long enough that the
/// instrumented child has reached its Ctrl-C listener.
#[cfg(unix)]
const SIGINT_READY_WAIT: std::time::Duration = std::time::Duration::from_millis(1200);

#[cfg(unix)]
fn killed_by_signal(status: &std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt as _;
    format!(
        "the child was killed by signal {:?} before it installed its Ctrl-C \
         listener (a loaded machine widens that window to seconds)",
        status.signal()
    )
}

/// Spawn the bridge, optionally feed it `stdin_bytes`, stop it with SIGINT, and
/// return `(exit status, captured stdout, captured stderr, its HOME)`.
///
/// Retries while the child was killed by the signal itself (it had not installed
/// its Ctrl-C listener yet — see [`sigint_until_exit`]), with a longer margin
/// each time. A child that exits by itself with a non-zero code is returned on
/// the first attempt, so a real regression fails every attempt and is reported
/// by the caller; the retry only absorbs the startup window a loaded machine
/// widens to seconds.
#[cfg(unix)]
fn sigint_scenario(
    label: &str,
    config: &str,
    stdin_bytes: Option<&[u8]>,
) -> (std::process::ExitStatus, String, String, std::path::PathBuf) {
    use std::io::Write as _;
    let mut wait = SIGINT_READY_WAIT;
    loop {
        let home = isolated_home(label);
        write_config(&home, config);
        let mut child = bin()
            .env("HOME", &home)
            .stdin(if stdin_bytes.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn binary");
        if let Some(bytes) = stdin_bytes {
            let stdin = child.stdin.as_mut().expect("piped stdin");
            stdin.write_all(bytes).unwrap();
            stdin.flush().unwrap();
            // Close the write end: the terminal channel pumps stdin on a
            // blocking-pool thread, and the runtime's teardown *waits* for that
            // thread. Leaving the pipe open makes the child hang during its own
            // shutdown (observed with `sample`: `BlockingPool::shutdown` parked
            // on the stdin read) even though the Ctrl-C path itself finished.
            drop(child.stdin.take());
        }
        // Drain the pipe on its own thread: what the child writes must never
        // fill the buffer and wedge it before it can shut down.
        let mut pipe = child.stdout.take().expect("piped stdout");
        let reader = std::thread::spawn(move || {
            use std::io::Read as _;
            let mut captured = Vec::new();
            let _ = pipe.read_to_end(&mut captured);
            captured
        });
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr_reader = std::thread::spawn(move || {
            use std::io::Read as _;
            let mut captured = String::new();
            stderr.read_to_string(&mut captured).expect("read stderr");
            captured
        });
        std::thread::sleep(wait);
        let status = sigint_until_exit(&mut child);
        let captured = reader.join().expect("stdout reader");
        let stderr = stderr_reader.join().expect("stderr reader");
        if status.success() || status.code().is_some() {
            return (
                status,
                String::from_utf8_lossy(&captured).into_owned(),
                stderr,
                home,
            );
        }
        wait += SIGINT_READY_WAIT * 3;
        assert!(
            wait < std::time::Duration::from_secs(30),
            "{}",
            killed_by_signal(&status)
        );
    }
}

/// Spawn the binary with the given config, wait for startup, send SIGINT,
/// and assert a clean exit(0) (the ctrl-c → shutdown path).
#[cfg(unix)]
fn sigint_shutdown_case(label: &str, config: &str) {
    let (status, stdout, stderr, _home) = sigint_scenario(label, config, None);
    assert!(
        status.success(),
        "clean shutdown expected, got {status:?}; stdout: {stdout}; stderr: {stderr}"
    );
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

/// Slack's Events API fallback in the **binary**.
///
/// Inbound webhook servers only exist in the non-test build: the in-process
/// webhook tests arm an ephemeral port through a `#[cfg(test)]` hook that the
/// shipped binary does not have. Starting the real one is what reaches that
/// hook's production twin (port from the config, no test slot) — so this is a
/// process test, and it waits for the server's own log line rather than racing a
/// fixed sleep. The Agent is unreachable on purpose: `auth.test` fails, is
/// warned about, and the events webhook still starts.
#[cfg(unix)]
#[test]
fn slack_events_webhook_starts_in_the_binary() {
    let (status, stdout) = sigint_after_stdout_marker(
        "slack-events-webhook",
        r#"{"agent": {"grpc_addr": "http://127.0.0.1:1"},
            "providers": {"slack": {"enabled": true, "bot_token": "xoxb-x",
                                    "signing_secret": "shh", "webhook_port": 0}}}"#,
        "events API webhook listening",
    );
    assert!(status.success(), "clean shutdown expected: {status:?}");
    // Ephemeral port: the configured 0 must have been used as given, and the
    // signature check is armed (the path is registered on the hook).
    assert!(
        stdout.contains("events API webhook listening"),
        "stdout: {stdout}"
    );
}

/// Spawn the bridge, wait until it logs `marker` on stdout (bounded), then stop
/// it with SIGINT and return `(exit status, captured stdout)`.
///
/// A fixed sleep races the asynchronous thing under test — here the inbound
/// webhook server, which starts after Slack's `auth.test` round trip — so this
/// waits for the component's own log line instead.
#[cfg(unix)]
fn sigint_after_stdout_marker(
    label: &str,
    config: &str,
    marker: &str,
) -> (std::process::ExitStatus, String) {
    let home = isolated_home(label);
    write_config(&home, config);
    let mut child = bin()
        .env("HOME", &home)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn binary");
    let stdout = child.stdout.take().expect("piped stdout");
    // Drain the pipe on its own thread (the child must never block on a full
    // buffer) and forward each line so this thread can look for the marker.
    let (lines_tx, lines_rx) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        use std::io::BufRead as _;
        let mut captured = String::new();
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let _ = lines_tx.send(line.clone());
            captured.push_str(&line);
            captured.push('\n');
        }
        captured
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut seen = false;
    while std::time::Instant::now() < deadline {
        match lines_rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(line) if line.contains(marker) => {
                seen = true;
                break;
            }
            // Another line, or nothing yet: keep waiting until the deadline.
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let status = sigint_until_exit(&mut child);
    let captured = reader.join().expect("stdout reader");
    assert!(seen, "the child never logged {marker:?}: {captured}");
    (status, captured)
}

/// Spawn the binary with piped stdin, feed it prompts, then SIGINT.
///
/// This is the only way to cover the terminal channel's read loop: in-process
/// the reader would block on the test runner's own stdin.
#[cfg(unix)]
#[test]
fn terminal_channel_reads_prompts_and_exits_cleanly() {
    let (status, stdout, stderr, home) = sigint_scenario(
        "cli-channel",
        r#"{"agent": {"grpc_addr": "http://127.0.0.1:1"},
            "providers": {"cli": {"enabled": true, "dm_policy": "open"}}}"#,
        // A blank line is skipped, the prompt is submitted, /quit ends the loop.
        Some(b"\nhello from the terminal\n/quit\n"),
    );
    assert!(
        status.success(),
        "clean shutdown expected: {status:?}; stdout: {stdout}; stderr: {stderr}"
    );
    // The agent is unreachable, so the terminal is told rather than left quiet.
    assert!(
        stdout.contains("Cannot reach the agent") || stdout.contains("unreachable"),
        "stdout: {stdout}"
    );
    // The bridge published a status snapshot while it ran.
    assert!(home.join(".future/channels/status.json").exists());
}

/// The same config with the terminal channel *disabled* exercises the
/// configured-but-off branch of the starter.
#[cfg(unix)]
#[test]
fn a_disabled_terminal_channel_does_not_start() {
    sigint_shutdown_case(
        "cli-channel-disabled",
        r#"{"providers": {"cli": {"enabled": false}}}"#,
    );
}

#[test]
fn the_channel_diagnostics_run_without_a_bridge() {
    // `list` / `status` are handled before the bridge starts, so they answer
    // with nothing running — including on a machine with no config at all.
    //
    // This binary is `future-channel`, which takes the subcommand as its own
    // first argument; the `channel` word belongs to the unified `future` CLI
    // that embeds it. Passing it here would fall through to the bridge, which
    // writes a default config and exits non-zero.
    let home = isolated_home("cli-diagnostics");
    for args in [vec!["list"], vec!["status"]] {
        let out = bin()
            .env("HOME", &home)
            .args(&args)
            .output()
            .expect("run binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{args:?} failed: {stdout}");
        assert!(!stdout.is_empty(), "{args:?} printed nothing");
    }
    // A missing config is reported, never written by a read-only command.
    assert!(!home.join(".future/channels/config.json").exists());

    let json = bin()
        .env("HOME", &home)
        .args(["status", "--format", "json"])
        .output()
        .expect("run binary");
    let stdout = String::from_utf8_lossy(&json.stdout);
    assert!(stdout.contains("bridgeRunning"), "{stdout}");
    assert!(stdout.contains("outboundQueue"), "{stdout}");
}

/// `--text -` reads the body from stdin, which only the process can do.
#[cfg(unix)]
#[test]
fn a_proactive_send_can_read_its_text_from_stdin() {
    use std::io::Write as _;

    let home = isolated_home("cli-send-stdin");
    write_config(
        &home,
        r#"{"providers": {"cli": {"enabled": true, "dm_policy": "open"}}}"#,
    );
    let mut child = bin()
        .env("HOME", &home)
        // This binary takes the subcommand as its own first argument (see the
        // note in `the_channel_diagnostics_run_without_a_bridge`).
        .args(["send", "--channel", "cli", "--to", "c1", "--text", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn binary");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(b"from the pipe\n")
        .unwrap();
    // Close the write end, or reading stdin would wait for more input forever.
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("cli: sent"), "{stdout}");
}

#[test]
fn testing_an_unknown_channel_fails_with_a_readable_error() {
    let home = isolated_home("cli-test-unknown");
    let out = bin()
        .env("HOME", &home)
        .args(["test", "not-a-channel"])
        .output()
        .expect("run binary");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown channel"), "{stderr}");
}

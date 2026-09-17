//! Subprocess checks use fresh HOME/USERPROFILE and never start or contact the
//! developer's Agent. First-time setup without a TTY must fail before spawning.
use std::path::Path;
use std::process::{Command, Stdio};

fn desktop(home: &Path) -> Command {
    isolated_command(env!("CARGO_BIN_EXE_futureos"), home)
}

// Run the standalone lifecycle checks with --no-default-features --features headless.
// No runtime flag is supplied: this binary must always start without a GUI.
#[cfg(feature = "headless")]
fn headless(home: &Path) -> Command {
    isolated_command(env!("CARGO_BIN_EXE_futureos-headless"), home)
}

fn isolated_command(binary: &str, home: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
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
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: futureos [--help]"));
    assert!(!home.path().join(".future").exists());
    for args in [
        vec!["--headless"],
        vec!["--headless", "--no-qr"],
        vec!["--re-pair"],
        vec!["--no-qr"],
        vec!["--help", "--headless"],
    ] {
        let invalid = desktop(home.path()).args(args).output().unwrap();
        assert_eq!(invalid.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("futureos-headless"));
        assert!(invalid.stdout.is_empty());
        assert!(!home.path().join(".future").exists());
    }
}

#[cfg(feature = "headless")]
#[test]
fn standalone_help_and_invalid_arguments_have_no_startup_side_effects() {
    for args in [vec!["--help"], vec!["-h"], vec!["--no-qr", "--help"]] {
        let home = tempfile::tempdir().unwrap();
        let help = headless(home.path()).args(args).output().unwrap();
        assert!(help.status.success());
        assert!(String::from_utf8_lossy(&help.stdout)
            .contains("Usage: futureos-headless [--no-qr] [--re-pair]"));
        assert!(!home.path().join(".future").exists());
    }
    for arg in ["--headless", "--gui", "--unknown", "-psn_123"] {
        let home = tempfile::tempdir().unwrap();
        let invalid = headless(home.path()).arg(arg).output().unwrap();
        assert_eq!(invalid.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("Unknown option"));
        assert!(!home.path().join(".future").exists());
    }
}

#[cfg(feature = "headless")]
#[test]
fn standalone_options_do_not_require_headless_flag() {
    for args in [
        vec!["--no-qr"],
        vec!["--re-pair", "--no-qr"],
        vec!["--re-pair"],
    ] {
        let home = tempfile::tempdir().unwrap();
        let result = headless(home.path()).args(args).output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("interactive terminal"), "{error}");
        assert!(result.stdout.is_empty());
        assert!(!home.path().join(".future/remote_pairing.json").exists());
        assert!(!home.path().join(".future/app/app.db").exists());
    }
}

#[cfg(feature = "headless")]
#[test]
fn first_setup_refuses_redirected_logs_without_printing_authorization_material() {
    let home = tempfile::tempdir().unwrap();
    let result = headless(home.path()).output().unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("interactive terminal"), "{error}");
    assert!(result.stdout.is_empty());
    assert!(!home.path().join(".future/remote_pairing.json").exists());
    assert!(!error.contains("Started the bundled Agent"));
}

#[cfg(feature = "headless")]
#[test]
fn occupied_data_directory_exits_cleanly_without_startup_side_effects() {
    let home = tempfile::tempdir().unwrap();
    let directory = home.path().join(".future/app");
    std::fs::create_dir_all(&directory).unwrap();
    let lock_path = directory.join("desktop.lock");
    let lock = std::fs::File::create(&lock_path).unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();

    let result = headless(home.path()).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("Desktop is already running"), "{error}");
    assert!(error.contains("Ctrl+C"), "{error}");
    assert!(!error.contains("panicked"), "{error}");
    assert!(!error.contains("os error"), "{error}");
    assert!(result.stdout.is_empty());
    assert!(!directory.join("app.db").exists());
    // A rejected second process must not unlink or release the owner's lock.
    let second = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(fs2::FileExt::try_lock_exclusive(&second).is_err());
    // Other parallel tests may briefly inherit this descriptor between fork
    // and exec. Unlock explicitly rather than waiting for every copy to close.
    fs2::FileExt::unlock(&lock).unwrap();
    fs2::FileExt::try_lock_exclusive(&second).unwrap();
}

#[cfg(feature = "headless")]
#[test]
fn corrupt_credentials_are_not_overwritten_or_treated_as_signed_out() {
    let home = tempfile::tempdir().unwrap();
    let directory = home.path().join(".future/agent");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("auth.json");
    let original = "this is deliberately not JSON";
    std::fs::write(&path, original).unwrap();
    let result = headless(home.path()).output().unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    assert!(result.stdout.is_empty());
}

#[cfg(not(feature = "gui"))]
#[test]
fn gui_free_build_points_to_standalone_entrypoint() {
    let home = tempfile::tempdir().unwrap();
    let result = desktop(home.path()).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("futureos-headless"));
    assert!(!home.path().join(".future").exists());
}

#[cfg(all(unix, feature = "headless"))]
#[test]
fn terminal_signals_cancel_startup_and_leave_the_external_endpoint_alone() {
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    for signal in ["-INT", "-TERM", "-HUP"] {
        let home = tempfile::tempdir().unwrap();
        // Every endpoint is local and owned by this test, even if the client
        // considers the TCP connection ready before HTTP/2 answers arrive.
        let endpoint = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = endpoint.local_addr().unwrap();
        let agent_dir = home.path().join(".future/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("auth.json"),
            serde_json::json!({"future":{"key":"test-only", "base_url":format!("http://{address}/api")}}).to_string(),
        )
        .unwrap();
        let pairing = home.path().join(".future/remote_pairing.json");
        let original = r#"{"handshakeVersion":1,"pairId":"test","desktopId":"desktop_test","nkeySeed":"test-only","userJwt":"test-only","natsUrl":"test-only","natsWsUrl":"test-only","jwtExpiresAt":1}"#;
        std::fs::write(&pairing, original).unwrap();
        // This fresh socket deliberately never answers HTTP/2. It cannot be a
        // real Agent, and startup never reaches an external platform request.
        let mut guard = ChildGuard(
            headless(home.path())
                .env("FUTURE_AGENT_GRPC_ADDR", format!("http://{address}"))
                .spawn()
                .unwrap(),
        );
        let child = &mut guard.0;
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        assert!(line.contains("foreground mode"), "{line}");
        let sent = Command::new("kill")
            .args([signal, &child.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
        let deadline = Instant::now() + Duration::from_secs(8);
        let exit = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("headless Desktop did not exit after {signal}");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(exit.success(), "{signal}: {exit}");
        assert_eq!(endpoint.local_addr().unwrap(), address);
        assert_eq!(std::fs::read_to_string(&pairing).unwrap(), original);
    }
}

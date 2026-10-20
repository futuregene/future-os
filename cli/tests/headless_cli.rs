//! Subprocess checks use fresh HOME/USERPROFILE and never start or contact the
//! developer's Agent. First-time setup without a TTY must fail before spawning.
use std::path::Path;
use std::process::{Command, Stdio};

// Run the actual unified CLI headless subcommand with no Desktop dependency.
fn headless(home: &Path) -> Command {
    {
        let mut command = isolated_command(env!("CARGO_BIN_EXE_future"), home);
        command.arg("headless");
        command
    }
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
fn standalone_help_and_invalid_arguments_have_no_startup_side_effects() {
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["--relay", "https://relay.example", "--help"],
    ] {
        let home = tempfile::tempdir().unwrap();
        let help = headless(home.path()).args(args).output().unwrap();
        assert!(help.status.success());
        // The usage line names every flag the entry point accepts; asserting a
        // prefix keeps the test about "help was printed" rather than about the
        // exact option list.
        assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: future headless"));
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

/// `--relay URL` is accepted in both spellings and never reaches the platform.
/// This only asserts the parse (an unattended run would block on the terminal
/// rule); the end-to-end path is `remote::relay_e2e`.
#[test]
fn relay_flags_are_accepted_in_both_spellings() {
    for args in [
        vec!["--relay", "https://relay.example", "--help"],
        vec!["--relay=https://relay.example", "--help"],
        vec![
            "--relay",
            "https://relay.example",
            "--relay-ca",
            "/tmp/ca.pem",
            "--help",
        ],
    ] {
        let home = tempfile::tempdir().unwrap();
        let help = headless(home.path()).args(&args).output().unwrap();
        assert!(help.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: future headless"));
    }
    // A flag without its value is an error, not a silent no-op.
    for args in [vec!["--relay"], vec!["--relay-ca"]] {
        let home = tempfile::tempdir().unwrap();
        let refused = headless(home.path()).args(&args).output().unwrap();
        assert_eq!(refused.status.code(), Some(1), "{args:?}");
        assert!(String::from_utf8_lossy(&refused.stderr).contains("needs a value"));
    }
}

/// The headless entry point starts with no runtime flag at all, and first-time
/// setup refuses a redirected terminal *before* any state is touched.
#[test]
fn a_plain_launch_needs_no_headless_flag() {
    let home = tempfile::tempdir().unwrap();
    let result = headless(home.path()).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("interactive terminal"), "{error}");
    assert!(result.stdout.is_empty());
    assert!(!home.path().join(".future/remote_pairing.json").exists());
    assert!(!home.path().join(".future/app/app.db").exists());
}

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

#[cfg(unix)]
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
        // Environment hydration may log before the foreground marker. Drain
        // stderr continuously and wait for the marker with a deadline, rather
        // than depending on the first line or blocking forever in read_line.
        let stderr = child.stderr.take().unwrap();
        let (lines, received) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let _ = lines.send(line);
            }
        });
        let ready_deadline = Instant::now() + Duration::from_secs(15);
        let mut startup_log = String::new();
        loop {
            let line = received
                .recv_timeout(ready_deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!("headless foreground marker was not received: {error}\n{startup_log}")
                });
            startup_log.push_str(&line);
            startup_log.push('\n');
            if line.contains("foreground mode") {
                break;
            }
        }
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
        reader.join().unwrap();
        for line in received.try_iter() {
            startup_log.push_str(&line);
            startup_log.push('\n');
        }
        assert!(exit.success(), "{signal}: {exit}\n{startup_log}");
        assert_eq!(endpoint.local_addr().unwrap(), address);
        assert_eq!(std::fs::read_to_string(&pairing).unwrap(), original);
    }
}

// ---------------------------------------------------------------------------
// The foreground session of the *real* server binary: login -> embedded Agent ->
// remote supervision (`src/headless/mod.rs::session`).
//
// `src/headless/mod.rs` has a `#[cfg(test)] mod tests` that can drive the parts
// of that module which take their inputs as parameters (`ensure_login`,
// `wait_for_login`, `print_invitation`, …). The session body is different: it is
// the entry point itself — it reads the real account file, starts the real
// embedded Agent, spawns the background observers and then supervises the remote
// bridge for the lifetime of the process. No unit test can run it, because the
// pieces it drives (`agent_host::host()`, the remote supervisor's process-wide
// `SUPERVISOR`, the shutdown admission flag) are per-process singletons whose
// state would leak into every other test in the lib target.
//
// Spawning the built binary is therefore the only way in, and it is the honest
// one: it exercises the `#[cfg(not(test))]` halves of the crate at the same
// time. The platform API is served from a loopback listener owned by the test,
// so both arms below are deterministic and offline.
// ---------------------------------------------------------------------------

/// What a loopback platform answers on its *pairing* endpoints. Both modes serve
/// the account profile, because passing that check is what lets the session get
/// past login at all.
#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Platform {
    /// The platform is reachable, but its pairing service fails with a 5xx. The
    /// remote supervisor categorizes that as a server fault, so the entry point
    /// must keep running in a deferred phase instead of exiting.
    PairingDown,
    /// The platform says the stored pairing credential is revoked.
    PairingRevoked,
}

/// A loopback platform API. Every request is answered — with the status the arm
/// asks for — so nothing in the session can block on a half-open connection.
///
/// It must answer for as long as the arm runs: a listener that dies mid-run
/// turns the arm under test into a *different* arm (a revoked pairing becomes an
/// unreachable platform), which is how this fixture's first version failed — the
/// run silently took the deferred path and the assertion looked like a timeout.
/// So transient `accept` errors are recorded and retried rather than returning,
/// and each connection is served on its own thread, so one stalled connection
/// cannot hold up the next request. The recorded paths also let an arm assert
/// *which* endpoints the run really called, and put the reason in a failure.
#[cfg(unix)]
struct FakePlatform {
    url: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    acceptor: Option<std::thread::JoinHandle<()>>,
    served: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    errors: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[cfg(unix)]
impl FakePlatform {
    fn start(mode: Platform) -> Self {
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let served = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (flag, paths, failures) = (stop.clone(), served.clone(), errors.clone());
        let acceptor = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let paths = paths.clone();
                        let failures = failures.clone();
                        std::thread::spawn(move || {
                            if let Err(error) = answer(stream, mode, &paths) {
                                failures.lock().unwrap().push(error);
                            }
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => {
                        failures.lock().unwrap().push(format!("accept: {error}"));
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
            }
        });
        Self {
            url,
            stop,
            acceptor: Some(acceptor),
            served,
            errors,
        }
    }

    /// The request paths this platform answered, in order.
    fn served(&self) -> Vec<String> {
        self.served.lock().unwrap().clone()
    }

    /// One line naming what the platform did, for a failing assertion.
    fn diagnostics(&self) -> String {
        format!(
            "platform served {:?}; accept/handler errors {:?}",
            self.served(),
            self.errors.lock().unwrap()
        )
    }
}

/// Answer one request: the account profile always succeeds (it is the check the
/// session must pass to get past login), everything else follows the arm.
///
/// The accepted stream is put back into **blocking** mode first: on macOS a
/// socket accepted from a non-blocking listener inherits `O_NONBLOCK`, so a
/// `read` that arrives before the client's bytes does returns `EAGAIN`
/// (`Resource temporarily unavailable`) instead of waiting — and a handler that
/// treats that as fatal closes the connection, which the client reports as
/// "error sending request" and which silently changes which arm the run takes.
#[cfg(unix)]
fn answer(
    mut stream: std::net::TcpStream,
    mode: Platform,
    served: &std::sync::Mutex<Vec<String>>,
) -> Result<(), String> {
    use std::io::{Read, Write};
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("blocking mode: {error}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    let mut request = [0_u8; 8192];
    let read = stream
        .read(&mut request)
        .map_err(|error| format!("read request: {error}"))?;
    if read == 0 {
        return Err("connection closed before a request arrived".to_string());
    }
    let head = String::from_utf8_lossy(&request[..read]).into_owned();
    let path = head
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let (status, body) = if path.ends_with("/client/v1/account/profile") {
        ("200 OK", r#"{"email":"operator@example.com"}"#)
    } else {
        match mode {
            Platform::PairingDown => (
                "500 Internal Server Error",
                r#"{"message":"pairing service unavailable"}"#,
            ),
            Platform::PairingRevoked => (
                "401 Unauthorized",
                r#"{"error":"invalid_remote_credential","message":"gone"}"#,
            ),
        }
    };
    served.lock().unwrap().push(path);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("write response: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("flush response: {error}"))
}

#[cfg(unix)]
impl Drop for FakePlatform {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(acceptor) = self.acceptor.take() {
            let _ = acceptor.join();
        }
    }
}

/// The state a server reaches after one interactive setup: a signed-in account
/// and a saved v2 pairing. Written by hand rather than through the app, which is
/// what keeps these runs offline; the NKey seed is generated here because the
/// credential refresh signs with it.
#[cfg(unix)]
fn signed_in_home(home: &Path, platform: &str) {
    let directory = home.join(".future");
    std::fs::create_dir_all(directory.join("agent")).unwrap();
    std::fs::write(
        directory.join("agent/auth.json"),
        serde_json::json!({
            "future": { "key": "test-only", "base_url": format!("{platform}/api") },
        })
        .to_string(),
    )
    .unwrap();
    let seed = nkeys::KeyPair::new_user()
        .seed()
        .expect("user seed")
        .to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    std::fs::write(
        directory.join("remote_pairing.json"),
        serde_json::json!({
            "handshakeVersion": 2,
            // Only its presence is read before the credential refresh, which the
            // platform answers; the crypto material is never used in these arms.
            "secure": {
                "privateKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "publicKey": "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
                "secret": "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC",
                "expiresAt": now + 600,
            },
            "pairId": "pair_test",
            "desktopId": "desktop_test",
            "nkeySeed": seed,
            "userJwt": "test-only",
            "natsUrl": "nats://127.0.0.1:1",
            "natsWsUrl": "ws://127.0.0.1:1",
            "jwtExpiresAt": now + 3600,
        })
        .to_string(),
    )
    .unwrap();
}

/// The foreground server: no external Agent endpoint is configured, so startup
/// takes the embedded-Agent path instead of attaching to a dead address.
#[cfg(unix)]
fn foreground(home: &Path) -> Command {
    let mut command = headless(home);
    command.env_remove("FUTURE_AGENT_GRPC_ADDR");
    command
}

/// Kill the child if the test fails early, so a red test never leaves a server
/// process (and its embedded Agent) behind.
#[cfg(unix)]
struct ChildGuard(std::process::Child);

#[cfg(unix)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// The child's stderr, drained on a thread: a startup failure must be reported
/// with the log rather than blocking the test in a `read_line`.
#[cfg(unix)]
struct Stderr {
    lines: std::sync::mpsc::Receiver<String>,
    collected: String,
}

#[cfg(unix)]
impl Stderr {
    fn take(child: &mut std::process::Child) -> Self {
        use std::io::{BufRead, BufReader};
        let stderr = child.stderr.take().unwrap();
        let (lines, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let _ = lines.send(line);
            }
        });
        Self {
            lines: received,
            collected: String::new(),
        }
    }

    fn drain(&mut self) {
        for line in self.lines.try_iter() {
            self.collected.push_str(&line);
            self.collected.push('\n');
        }
    }

    /// Drain until `needle` is reported, or fail with what was seen so far.
    fn wait_for(&mut self, needle: &str, budget: std::time::Duration) {
        let deadline = std::time::Instant::now() + budget;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self.lines.recv_timeout(remaining).unwrap_or_else(|error| {
                panic!(
                    "{needle:?} was never reported ({error}):\n{}",
                    self.collected
                )
            });
            self.collected.push_str(&line);
            self.collected.push('\n');
            if line.contains(needle) {
                return;
            }
        }
    }
}

#[cfg(unix)]
fn wait_for_exit(
    child: &mut std::process::Child,
    log: &mut Stderr,
    platform: &FakePlatform,
    budget: std::time::Duration,
) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            log.drain();
            return status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            log.drain();
            panic!(
                "the run did not exit within {budget:?}:\n{}\n{}",
                log.collected,
                platform.diagnostics()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn signal(child: &std::process::Child, signal: &str) {
    let sent = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success(), "signal {signal} was not delivered");
}

/// A paired, signed-in server whose platform is reachable but whose *pairing*
/// service fails must keep serving in a deferred phase — and must still stop
/// cleanly on the operator's signal. This is the arm a server box runs in when
/// the platform has a bad day, and it is the one the entry point's own comment
/// describes ("an unreachable platform is not a login change").
#[cfg(unix)]
#[test]
fn a_deferred_remote_start_keeps_serving_and_stops_on_a_signal() {
    use std::time::Duration;

    let home = tempfile::tempdir().unwrap();
    let platform = FakePlatform::start(Platform::PairingDown);
    signed_in_home(home.path(), &platform.url);
    let pairing = home.path().join(".future/remote_pairing.json");
    let saved = std::fs::read_to_string(&pairing).unwrap();

    let mut guard = ChildGuard(foreground(home.path()).spawn().unwrap());
    let child = &mut guard.0;
    let mut log = Stderr::take(child);

    // The whole preamble has to have run for this line to appear: the account
    // file was read, the embedded Agent started, the profile check passed.
    log.wait_for("Platform login authorized", Duration::from_secs(300));
    // …and this one proves the remote supervision loop was entered, with the
    // phase the platform's failure must produce rather than a terminal one.
    log.wait_for("Remote: Reconnecting", Duration::from_secs(300));

    // A single iteration would also print that line; the loop is what a server
    // needs, so require the process to still be alive a few sleeps later.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        child.try_wait().unwrap().is_none(),
        "a deferred remote start must keep serving:\n{}",
        log.collected
    );
    // …and the deferred phase must be the platform's *answer*, not a listener
    // that went away: the run has to have reached the pairing endpoint.
    let served = platform.served();
    assert!(
        served
            .iter()
            .any(|path| path.ends_with("/client/v1/remote/auth/token")),
        "the deferred phase must come from the pairing endpoint: {served:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&pairing).unwrap(),
        saved,
        "a deferred start must not rewrite or drop a usable pairing"
    );
    log.drain();
    assert!(
        !log.collected.contains("PHONE PAIRING"),
        "without a terminal no invitation may be printed:\n{}",
        log.collected
    );

    // The operator's way out is the signal the entry point documents.
    signal(child, "-INT");
    let exit = wait_for_exit(child, &mut log, &platform, Duration::from_secs(120));
    assert!(
        exit.success(),
        "SIGINT must stop the run cleanly ({exit}):\n{}",
        log.collected
    );
}

/// The account this run serves is pinned at authorization (`account_key`) and
/// re-read from `auth.json` on every supervision iteration, so a login that is
/// removed or replaced underneath a live server must end the run with *that*
/// reason — otherwise the old account's remote ingress stays open until its JWT
/// expires.
///
/// That arm is only reachable from the real entry point: the pinned account is
/// read from the real file at process start and the loop is the entry point's
/// own. This therefore spawns the binary, waits until the supervision loop is
/// demonstrably running, and rotates the stored key the way a second login
/// would. It asserts the arm's effect (the entry point stops serving with that
/// exact reason and exit code) rather than that "a rewrite happened" — a run
/// that ignored the change would keep the deferred phase and hit the budget.
#[cfg(unix)]
#[test]
fn a_login_change_while_supervising_stops_the_run_with_that_reason() {
    use std::time::Duration;

    let home = tempfile::tempdir().unwrap();
    let platform = FakePlatform::start(Platform::PairingDown);
    signed_in_home(home.path(), &platform.url);
    let auth = home.path().join(".future/agent/auth.json");
    let authorized = std::fs::read_to_string(&auth).unwrap();

    let mut guard = ChildGuard(foreground(home.path()).spawn().unwrap());
    let child = &mut guard.0;
    let mut log = Stderr::take(child);

    // The whole preamble has to have run before either line appears: the account
    // file was read, the embedded Agent started, the profile check passed, and
    // the remote supervision loop produced its first phase.
    log.wait_for("Platform login authorized", Duration::from_secs(300));
    log.wait_for("Remote: Reconnecting", Duration::from_secs(300));
    // A deferred phase is not terminal, so the run is still serving here — which
    // is what makes the exit below the rotation's effect and not a startup
    // failure that happened to arrive late.
    assert!(
        child.try_wait().unwrap().is_none(),
        "a deferred remote start must still be serving before the login changes:\n{}",
        log.collected
    );

    // Rotate the key atomically: a partially written file would be a corrupt-
    // credentials arm (a different reason), not a login change.
    let rotated = serde_json::json!({
        "future": { "key": "test-only-rotated", "base_url": format!("{}/api", platform.url) },
    })
    .to_string();
    let staging = home.path().join(".future/agent/auth.json.rotated");
    std::fs::write(&staging, &rotated).unwrap();
    std::fs::rename(&staging, &auth).unwrap();

    let status = wait_for_exit(child, &mut log, &platform, Duration::from_secs(30));
    assert_eq!(
        status.code(),
        Some(1),
        "a changed login must fail the entry point:\n{}",
        log.collected
    );
    // The arm's own effect, spelled exactly: this is the message that says the
    // pinned account is no longer the one on disk. Neither the revoked-pairing
    // arm nor a startup failure can produce it.
    assert!(
        log.collected
            .contains("Platform login changed or was removed. Remote access is closing"),
        "the exit must name the login change:\n{}",
        log.collected
    );
    assert!(
        !log.collected.contains("Remote connection stopped"),
        "the stop must come from the login check, not from a remote phase:\n{}",
        log.collected
    );
    // The account that was serving is the original one: this line is what proves
    // the arm compared against a pinned key rather than starting up already
    // signed out. (`current_platform_url` strips the `/api` suffix, so the line
    // carries the bare loopback origin.)
    assert!(
        log.collected
            .contains(&format!("Platform login authorized ({}).", platform.url)),
        "the run must have authorized the original account first:\n{}",
        log.collected
    );
    assert!(
        !authorized.contains("test-only-rotated"),
        "the fixture must rotate away from the account it authorized"
    );
    // A stop must not silently repair or rewrite the login it just refused, and
    // it must not print an invitation for the account it is abandoning.
    assert_eq!(std::fs::read_to_string(&auth).unwrap(), rotated);
    assert!(
        !log.collected.contains("PHONE PAIRING"),
        "a stopped run may not print an invitation:\n{}",
        log.collected
    );
}

/// The other terminal phase: a pairing the platform has revoked cannot be used,
/// and the entry point must say so and stop — not serve an unusable credential
/// and not hide the reason behind a generic exit.
#[cfg(unix)]
#[test]
fn a_revoked_pairing_stops_the_run_with_the_reason_and_drops_the_credential() {
    use std::time::Duration;

    let home = tempfile::tempdir().unwrap();
    let platform = FakePlatform::start(Platform::PairingRevoked);
    signed_in_home(home.path(), &platform.url);
    let pairing = home.path().join(".future/remote_pairing.json");

    let mut guard = ChildGuard(foreground(home.path()).spawn().unwrap());
    let child = &mut guard.0;
    let mut log = Stderr::take(child);
    let status = wait_for_exit(child, &mut log, &platform, Duration::from_secs(300));

    // The arm is only meaningful if the platform really answered the two pairing
    // calls; a listener that died mid-run would otherwise look like this arm.
    let served = platform.served();
    assert!(
        served
            .iter()
            .any(|path| path.ends_with("/client/v1/remote/auth/token")),
        "the revoked arm must have asked for a credential refresh: {served:?}"
    );
    assert!(
        log.collected.contains("Platform login authorized"),
        "the stop must come from the pairing, not from login:\n{}",
        log.collected
    );
    assert_eq!(
        status.code(),
        Some(1),
        "a revoked pairing must fail the entry point:\n{}",
        log.collected
    );
    assert!(
        log.collected.contains("Remote connection stopped")
            && log.collected.contains("CredentialRevoked"),
        "the reason must be named:\n{}",
        log.collected
    );
    assert!(
        !log.collected.contains("PHONE PAIRING"),
        "a revoked pairing must not print an invitation without a terminal:\n{}",
        log.collected
    );
    assert!(
        !pairing.exists(),
        "an unusable credential must be dropped instead of served forever"
    );
}

/// The agent-mode guard is a safety property, not a formatting detail: `auto`
/// may fall back to an embedded Agent, `external` may **never** start one
/// ("attach-only mode never starts an embedded service"), and an unrecognised
/// value is a configuration error rather than a guess. Each refusal must reach
/// the operator as the reason, before any Agent comes up.
#[cfg(unix)]
#[test]
fn a_non_auto_agent_mode_never_starts_an_embedded_agent() {
    use std::time::Duration;

    for (mode, expected) in [
        (
            "external",
            "attach-only mode never starts an embedded service",
        ),
        (
            "bogus",
            "FUTURE_DESKTOP_AGENT_MODE must be auto or external",
        ),
    ] {
        let home = tempfile::tempdir().unwrap();
        let platform = FakePlatform::start(Platform::PairingDown);
        signed_in_home(home.path(), &platform.url);

        let mut guard = ChildGuard(
            foreground(home.path())
                .env("FUTURE_DESKTOP_AGENT_MODE", mode)
                .spawn()
                .unwrap(),
        );
        let child = &mut guard.0;
        let mut log = Stderr::take(child);
        let status = wait_for_exit(child, &mut log, &platform, Duration::from_secs(300));

        assert_eq!(
            status.code(),
            Some(1),
            "agent mode {mode:?} must fail the run:\n{}",
            log.collected
        );
        assert!(
            log.collected.contains(expected),
            "agent mode {mode:?} must name its refusal ({expected}):\n{}",
            log.collected
        );
        // The refusal precedes login: no platform request was ever made for a
        // run that cannot have an Agent to serve it.
        assert!(
            !log.collected.contains("Platform login authorized"),
            "agent mode {mode:?} must fail before login:\n{}",
            log.collected
        );
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn a_sandbox_helper_request_is_dispatched_before_any_startup() {
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_future"))
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

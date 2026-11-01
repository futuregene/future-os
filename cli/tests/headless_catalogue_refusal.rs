//! A model catalogue refresh the **Agent** refuses must be reported and then
//! ignored: the run keeps serving with the models it already has.
//!
//! `headless/mod.rs::refresh_catalogue` has two arms, and the one a passing
//! machine never takes is the error one. It cannot be reached by a *signed-out*
//! run either — the refresh only runs just after the platform account is
//! adopted — and an unreachable Agent is not a refusal: `sync_future_models`
//! answers a zeroed result for an Agent it cannot connect to, by contract. The
//! only input that reaches the arm is an Agent that answers and rejects the
//! command, which is what `future_host::test_support::agent`'s scripted mock
//! provides. So this test pins the Agent endpoint at that mock, serves the
//! platform's own profile endpoint from loopback, and runs the real binary.
//!
//! It lives in an integration target of its own because the mock points the
//! process-wide `FUTURE_AGENT_GRPC_ADDR` at itself: one test per process keeps
//! that global from being inherited by an unrelated test's child.

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// A loopback platform API that answers the account profile and refuses
    /// everything else. Only the profile endpoint is reached here — nothing is
    /// paired, so no pairing request is made — and replacing it with a refusal
    /// would put the run in the *signed-out* arm, which never refreshes a
    /// catalogue at all.
    struct ProfileOnlyPlatform {
        url: String,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        acceptor: Option<std::thread::JoinHandle<()>>,
    }

    impl ProfileOnlyPlatform {
        fn start() -> Self {
            use std::sync::atomic::Ordering;
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = stop.clone();
            let acceptor = std::thread::spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = answer(stream);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(20)),
                    }
                }
            });
            Self {
                url,
                stop,
                acceptor: Some(acceptor),
            }
        }
    }

    impl Drop for ProfileOnlyPlatform {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            if let Some(acceptor) = self.acceptor.take() {
                let _ = acceptor.join();
            }
        }
    }

    /// Answer one request on its own thread (a stalled connection must not stop
    /// the platform from answering anything else). The accepted socket is put
    /// back into blocking mode first: on macOS it inherits `O_NONBLOCK` from the
    /// listener, and a read that arrives before the client's bytes would fail
    /// with `EAGAIN` and silently close the connection.
    fn answer(mut stream: std::net::TcpStream) -> Result<(), String> {
        stream
            .set_nonblocking(false)
            .map_err(|error| format!("blocking mode: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| format!("read timeout: {error}"))?;
        let mut request = [0_u8; 8192];
        let read = stream
            .read(&mut request)
            .map_err(|error| format!("read request: {error}"))?;
        if read == 0 {
            return Ok(());
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
            ("500 Internal Server Error", r#"{"message":"not served"}"#)
        };
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

    /// The state one interactive setup leaves behind, minus the pairing: a
    /// signed-in account whose platform is this test's loopback listener.
    fn signed_in_home(home: &Path, platform: &str) -> std::path::PathBuf {
        let agent_dir = home.join(".future/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let auth = agent_dir.join("auth.json");
        std::fs::write(
            &auth,
            serde_json::json!({
                "future": { "key": "test-only", "base_url": format!("{platform}/api") },
            })
            .to_string(),
        )
        .unwrap();
        auth
    }

    /// Kill the child if the test fails early, so a red test never leaves a
    /// server process (and its Agent client) behind.
    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    /// The child's stderr, drained on a thread: a startup failure must be
    /// reported with the log rather than blocking the test in a `read_line`.
    struct Stderr {
        lines: std::sync::mpsc::Receiver<String>,
        collected: String,
    }

    impl Stderr {
        fn take(child: &mut Child) -> Self {
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
        fn wait_for(&mut self, needle: &str, budget: Duration) {
            let deadline = Instant::now() + budget;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
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

    fn wait_for_exit(
        child: &mut Child,
        log: &mut Stderr,
        budget: Duration,
    ) -> std::process::ExitStatus {
        let deadline = Instant::now() + budget;
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                log.drain();
                return status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                log.drain();
                panic!("the run did not exit within {budget:?}:\n{}", log.collected);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn a_catalogue_refresh_the_agent_refuses_is_reported_and_the_run_keeps_serving() {
        let platform = ProfileOnlyPlatform::start();
        let home = tempfile::tempdir().unwrap();
        let auth = signed_in_home(home.path(), &platform.url);
        let written = std::fs::read_to_string(&auth).unwrap();

        // A live Agent that answers every other command and refuses this one.
        let mock = future_host::test_support::agent::mock_agent();
        mock.reject_default(
            "sync_future_models",
            "the platform catalogue is unavailable",
        );
        let address = std::env::var("FUTURE_AGENT_GRPC_ADDR")
            .expect("the mock Agent must have published its endpoint");

        let mut guard = ChildGuard(
            Command::new(env!("CARGO_BIN_EXE_future"))
                .arg("host")
                .env("HOME", home.path())
                .env("USERPROFILE", home.path())
                .env("FUTURE_HOME", home.path().join(".future"))
                .env_remove("FUTURE_AGENT_SOCKET")
                .env("FUTURE_AGENT_GRPC_ADDR", &address)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let child = &mut guard.0;
        let mut log = Stderr::take(child);

        // The account is adopted from disk, so the run is past the only gate
        // that can precede the refresh. A run that did *not* sign in would never
        // refresh a catalogue, and its own sentence is what says so.
        log.wait_for("Platform account signed in", Duration::from_secs(300));
        let refused = "Model catalog refresh failed; existing configured models remain available: the platform catalogue is unavailable";
        log.wait_for(refused, Duration::from_secs(300));

        // Refused and ignored: the run reaches its remote supervision instead of
        // stopping over a catalogue, and no invitation is minted to get there.
        log.wait_for(
            "Preparing the remote connection...",
            Duration::from_secs(120),
        );
        log.drain();
        assert!(
            !log.collected.contains("Not signed in to FutureOS"),
            "a signed-in run must not report itself as signed out:\n{}",
            log.collected
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "a refused catalogue must not end the run:\n{}",
            log.collected
        );

        let sent = Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success(), "the stop signal was not delivered");
        let status = wait_for_exit(child, &mut log, Duration::from_secs(120));
        assert!(
            status.success(),
            "SIGINT must stop it cleanly: {status}\n{}",
            log.collected
        );

        // The failure was a property of the Agent's answer, not of this run's
        // own state: the credential the test wrote is still exactly there.
        assert_eq!(
            std::fs::read_to_string(&auth).unwrap(),
            written,
            "a refused catalogue refresh must not rewrite the account file"
        );

        let mut stdout = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            pipe.read_to_string(&mut stdout)
                .expect("read the child's stdout");
        }
        assert!(
            stdout.is_empty(),
            "a signed-in launch prints authorization material on stdout only when it has any; \
             this run minted none, so stdout must be empty: {stdout:?}"
        );
    }
}

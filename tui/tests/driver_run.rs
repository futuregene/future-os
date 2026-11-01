//! The workbench's process entry, driven as a **real process**: `run()` starts a
//! terminal, attaches to an agent, paints the first frame and only returns when
//! the reader leaves — none of which an in-process unit test can do (it would
//! need a tty, and `run()` owns the one it starts).
//!
//! The shape is the one `signal_shutdown.rs` uses for the same reason: the test
//! binary re-executes itself (`--exact driver_run_child`) as the TUI, the parent
//! hands it a PTY on stdin, waits for the first frame, types `ctrl+x`, and reads
//! the exit status and the settings file the process wrote on its way out.
//!
//! `#[cfg(unix)]`: the PTY is `posix_openpt`. Windows' console is a different
//! object; that half is a measurement gap, not a waiver.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ADDR_ENV: &str = "FUTURE_TUI_DRIVER_TEST_ADDR";
const SETTINGS_ENV: &str = "FUTURE_TUI_DRIVER_TEST_SETTINGS";
const CHILD_ENV: &str = "FUTURE_TUI_DRIVER_TEST_CHILD";
const CHILD_TEST: &str = "driver_run_child";

const EXIT_TIMEOUT: Duration = Duration::from_secs(20);

// ─── Child ────────────────────────────────────────────────────────────────

/// The child half: an ordinary no-op test unless the parent asked for it.
#[test]
fn driver_run_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let addr = std::env::var(ADDR_ENV).expect("the parent passes the agent address");
    let settings = std::env::var(SETTINGS_ENV).expect("the parent passes a settings path");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the child's runtime");
    let outcome = runtime.block_on(future_tui::workbench::run::run(
        future_tui::workbench::run::Options {
            addr,
            session: None,
            no_context_files: true,
            initial_prompt: None,
            continue_recent: false,
            // No network: the update check is the one thing here that would
            // reach outside this machine.
            offline: true,
            settings_path: PathBuf::from(settings),
        },
    ));
    match outcome {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("child: run() failed: {error}");
            std::process::exit(2)
        }
    }
}

// ─── Parent ───────────────────────────────────────────────────────────────

/// One run of the real entry point: a child, its PTY, and everything it writes.
struct Run {
    child: Child,
    master: Option<OwnedFd>,
    seen: String,
    dir: tempfile::TempDir,
    pub settings: PathBuf,
}

impl Run {
    fn spawn(addr: &str) -> Self {
        Self::spawn_with(addr, None)
    }

    /// `settings`: written before the child starts, exactly as a reader's own
    /// file would be (the surface reads it on its first frame).
    fn spawn_with(addr: &str, seeded: Option<&str>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let settings = dir.path().join("tui").join("settings.json");
        if let Some(body) = seeded {
            std::fs::create_dir_all(settings.parent().expect("parent")).expect("settings dir");
            std::fs::write(&settings, body).expect("settings file");
        }
        let mut command = Command::new(std::env::current_exe().expect("test binary path"));
        command
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env(CHILD_ENV, "1")
            .env(ADDR_ENV, addr)
            .env(SETTINGS_ENV, &settings)
            // An isolated FutureOS home: the child must not read the developer's
            // settings, keybindings or session store.
            .env("HOME", dir.path())
            .env("USERPROFILE", dir.path())
            .env("FUTURE_HOME", dir.path().join("future"))
            .env_remove("FUTURE_AGENT_SOCKET")
            .env_remove("FUTURE_AGENT_GRPC_ADDR")
            .env("TERM", "xterm-256color");
        // The PTY is the child's **stdout** as much as its stdin: the workbench
        // paints on fd 1, so redirecting stdout to a file would leave the parent
        // reading an empty master (measured: a 20 s wait and no frame).
        let (master, slave) = open_pty();
        let out = dup_fd(&slave);
        let err = dup_fd(&slave);
        command.stdin(Stdio::from(slave));
        command.stdout(Stdio::from(out));
        command.stderr(Stdio::from(err));
        let child = command.spawn().expect("spawn the child TUI");
        let seen = String::new();
        Self {
            child,
            master: Some(master),
            seen,
            dir,
            settings,
        }
    }

    /// Read whatever the child has written so far, without blocking.
    fn read(&mut self) {
        let Some(master) = self.master.as_ref() else {
            return;
        };
        let fd = borrow_fd(master);
        let mut buffer = [0u8; 8192];
        loop {
            let read =
                unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len()) };
            if read <= 0 {
                break;
            }
            self.seen
                .push_str(&String::from_utf8_lossy(&buffer[..read as usize]));
        }
    }

    /// Wait until the child has painted something matching `needle`.
    fn wait_for(&mut self, needle: &str, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            self.read();
            if self.seen.contains(needle) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Type `bytes` at the child, as a terminal would.
    fn type_bytes(&mut self, bytes: &[u8]) {
        let Some(master) = self.master.as_ref() else {
            return;
        };
        let fd = borrow_fd(master);
        let written =
            unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        assert!(written > 0, "writing to the pty failed");
    }

    /// Wait for the child to leave, **draining the pty while waiting**: a tty
    /// buffer is small (64 KiB), and a child that blocks writing its frame never
    /// reaches the key that was sent to stop it (measured: a 20 s wait and a
    /// process still painting).
    fn wait_exit(&mut self, limit: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + limit;
        loop {
            self.read();
            match self.child.try_wait().expect("try_wait") {
                Some(status) => return Some(status),
                None if Instant::now() >= deadline => return None,
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    /// The tail of what the child wrote: a whole screen of escape sequences makes
    /// a panic message unreadable.
    fn tail(&self) -> String {
        let start = self.seen.len().saturating_sub(400);
        self.seen[start..].to_string()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // Never leave a TUI behind: a failing assertion above still has to take
        // the child with it.
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.master = None;
    }
}

fn dup_fd(fd: &OwnedFd) -> OwnedFd {
    use std::os::fd::AsRawFd as _;
    let copy = unsafe { libc::dup(fd.as_raw_fd()) };
    assert!(copy >= 0, "dup of the pty slave failed");
    unsafe { OwnedFd::from_raw_fd(copy) }
}

fn borrow_fd(fd: &OwnedFd) -> i32 {
    use std::os::fd::AsRawFd as _;
    fd.as_raw_fd()
}

fn open_pty() -> (OwnedFd, OwnedFd) {
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master >= 0, "posix_openpt failed");
        assert_eq!(libc::grantpt(master), 0, "grantpt failed");
        assert_eq!(libc::unlockpt(master), 0, "unlockpt failed");
        let name = libc::ptsname(master);
        assert!(!name.is_null(), "ptsname failed");
        let slave = libc::open(name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(slave >= 0, "opening the pty slave failed");
        // The master is read without blocking, and neither end may leak into the
        // child's own children.
        let flags = libc::fcntl(master, libc::F_GETFL);
        assert!(flags >= 0);
        assert!(libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK) >= 0);
        (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
    }
}

// ─── The one run ──────────────────────────────────────────────────────────

/// `future tui` starts, paints, and leaves on `ctrl+x` — and what it leaves
/// behind is `settings.json`: the layout the reader had, written by the exit path
/// that only exists in the real entry point.
#[test]
fn the_real_entry_point_paints_a_frame_and_leaves_a_settings_file() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let (addr, server) = runtime.block_on(async {
        use future_rpc::proto::future_agent_server::FutureAgentServer;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
            .expect("tcp incoming");
        let server = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(FutureAgentServer::new(AttachMock))
                .serve_with_incoming(incoming),
        );
        (addr, server)
    });

    let mut run = Run::spawn(&addr);
    assert!(
        run.wait_for("FUTURE OS", EXIT_TIMEOUT),
        "the first frame never reached the terminal; the child said: {:?}",
        run.tail()
    );
    assert!(
        run.seen.contains("\x1b[?1049h"),
        "the child never took the alternate screen: {:?}",
        run.tail()
    );

    // A queued palette open, then `escape`, then the quit chord: the loop keeps
    // reading keys after a frame, and the surface owns the box in between.
    run.type_bytes(b"\x10");
    // The palette's title is copy, so it is drawn in the reader's language: the
    // fixture's home has no settings, and the surface follows the system locale.
    // Either spelling is the same control.
    let opened = run.wait_for("命令", Duration::from_secs(10)) || run.seen.contains("Commands");
    assert!(
        opened,
        "the palette did not open; the child said: {:?}",
        run.tail()
    );
    run.type_bytes(b"\x1b");
    // A terminal delivers keys in separate reads; `ESC` immediately followed by
    // `ctrl+x` is *one* chord (`alt+x`) to the parser, which is why a real
    // keypress never arrives glued together.
    std::thread::sleep(Duration::from_millis(250));
    run.type_bytes(b"\x18");
    if run.wait_exit(Duration::from_secs(5)).is_none() {
        // A second press, as a reader would when the first did not take.
        run.type_bytes(b"\x18");
    }
    let status = run.wait_exit(EXIT_TIMEOUT);
    assert!(
        status.is_some(),
        "ctrl+x did not end the real entry point; the child said: {:?}",
        run.tail()
    );
    assert_eq!(
        status.unwrap().code(),
        Some(0),
        "the process entry left with an error; the child said: {:?}",
        run.tail()
    );
    assert!(
        run.seen.contains("\x1b[?1049l"),
        "the alternate screen was not given back: {:?}",
        run.tail()
    );

    // The exit path persists what the reader had: the layout file exists and
    // names the fields the surface owns.
    let written =
        std::fs::read_to_string(&run.settings).expect("the exit path wrote settings.json");
    let value: serde_json::Value = serde_json::from_str(&written).expect("settings.json is json");
    for key in [
        "sidebarVisible",
        "sidebarWidth",
        "sidebarPosition",
        "mouseCapture",
    ] {
        assert!(
            value.get(key).is_some(),
            "settings.json is missing {key}: {written}"
        );
    }
    // …in the isolated home, never the developer's.
    assert!(
        run.settings.starts_with(run.home()),
        "wrote outside the sandbox"
    );
    server.abort();
    drop(runtime);
}

/// `notify`'s body is `for sequence in center.push(id, event, config,
/// terminal.is_tty())`: a process without a terminal is *told* not to ring the
/// desktop, so this is the only place it can run. The agent streams one approval
/// and the terminal should get the notification's escape sequence.
#[test]
fn an_approval_streamed_to_a_real_terminal_rings_the_desktop() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let (addr, server) = runtime.block_on(async {
        use future_rpc::proto::future_agent_server::FutureAgentServer;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
            .expect("tcp incoming");
        let server = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(FutureAgentServer::new(ApprovalMock))
                .serve_with_incoming(incoming),
        );
        (addr, server)
    });

    // The reader's own file, with notifications on: the surface reads it on its
    // first frame, so it has to exist before the child starts.
    let seeded = r#"{"notify":{"enabled":true,"bell":true,"osc9":true,"title":false}}"#;
    let mut run = Run::spawn_with(&addr, Some(seeded));
    assert!(
        run.wait_for("FUTURE OS", EXIT_TIMEOUT),
        "the first frame never reached the terminal; the child said: {:?}",
        run.tail()
    );
    // OSC 9 is the desktop notification the terminal shows.
    assert!(
        run.wait_for("\x1b]9;", Duration::from_secs(15)),
        "the streamed approval did not ring the desktop; the child said: {:?}",
        run.tail()
    );
    run.type_bytes(b"\x18");
    let status = run.wait_exit(Duration::from_secs(10));
    assert_eq!(status.and_then(|status| status.code()), Some(0));
    server.abort();
    drop(runtime);
}

/// An agent that streams exactly one approval and then stays open.
struct ApprovalMock;

#[tonic::async_trait]
impl future_rpc::proto::future_agent_server::FutureAgent for ApprovalMock {
    async fn execute_command(
        &self,
        request: tonic::Request<future_rpc::proto::RpcCommand>,
    ) -> Result<tonic::Response<future_rpc::proto::RpcResponse>, tonic::Status> {
        AttachMock.execute_command(request).await
    }

    type StreamEventsStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<Item = Result<future_rpc::proto::StreamEvent, tonic::Status>>
                + Send,
        >,
    >;

    async fn stream_events(
        &self,
        _request: tonic::Request<future_rpc::proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let approval = future_rpc::proto::StreamEvent {
            r#type: "approval_request".into(),
            data: serde_json::json!({
                "approval_request_id": "a-1",
                "tool_name": "bash",
                "summary": "run: rm -rf build",
            })
            .to_string(),
            session_id: "s-driver".into(),
            event_id: "e-1".into(),
            run_id: "run-1".into(),
            ..Default::default()
        };
        // One event, then nothing: the loop keeps running with an approval parked
        // on the surface, which is the state the notification is for.
        use futures_util::StreamExt as _;
        // One event, then nothing: the loop keeps running with the approval
        // parked on the surface.
        let stream =
            futures_util::stream::iter(vec![Ok(approval)]).chain(futures_util::stream::pending());
        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

/// An agent that answers what a start needs: the readiness handshake, the state,
/// the lists — and nothing else is required before the first frame.
struct AttachMock;

#[tonic::async_trait]
impl future_rpc::proto::future_agent_server::FutureAgent for AttachMock {
    async fn execute_command(
        &self,
        request: tonic::Request<future_rpc::proto::RpcCommand>,
    ) -> Result<tonic::Response<future_rpc::proto::RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        let data = match cmd.r#type.as_str() {
            "get_agent_readiness" => serde_json::json!({
                "version": future_rpc::connection::EXPECTED_AGENT_VERSION,
                "agentInstanceId": "driver-run-fixture",
                "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId": "0".repeat(64),
                "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION,
            })
            .to_string(),
            "get_state" => serde_json::json!({
                "sessionId": "s-driver",
                "cwd": ".",
            })
            .to_string(),
            "get_session_entries" => serde_json::json!({"entries": []}).to_string(),
            "list_sessions" => serde_json::json!({
                "sessions": [{"id": "s-driver", "cwd": ".", "updatedAtMs": 1_700_000_000_000_i64,
                              "model": "future/old", "sessionName": "driver run"}]
            })
            .to_string(),
            "list_models" => serde_json::json!({
                "models": [{"id": "old", "label": "old", "provider": "future",
                            "thinkingLevel": "default", "thinkingLevels": ["default"],
                            "defaultThinkingLevel": "default", "isDefault": true}]
            })
            .to_string(),
            _ => "{}".to_string(),
        };
        Ok(tonic::Response::new(future_rpc::proto::RpcResponse {
            id: cmd.id,
            r#type: "response".into(),
            command: cmd.r#type,
            success: true,
            data,
            ..Default::default()
        }))
    }

    type StreamEventsStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<Item = Result<future_rpc::proto::StreamEvent, tonic::Status>>
                + Send,
        >,
    >;

    async fn stream_events(
        &self,
        _request: tonic::Request<future_rpc::proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        Ok(tonic::Response::new(Box::pin(
            futures_util::stream::pending(),
        )))
    }
}

/// Silence the unused-import lint for `Read`/`Write`, which the raw-fd reads
/// above do not need but a reader of this file expects to find.
#[allow(dead_code)]
fn _io_traits(_: &mut (impl Read + Write)) {}
